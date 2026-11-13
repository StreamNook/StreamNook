//! Verified tokens: a token is used for an account only when Twitch says it
//! is that account's.
//!
//! [`TokenVerifier`] is pure over injected closures ([`VerifyIo`]): the app's
//! own token fetch for an account (already sorted into a token, a refusal or
//! "couldn't reach"), Twitch's validate call, a spawner, the token hash, the
//! clock and the account generation. The engine keeps one; each test builds
//! its own, so nothing here is global.
//!
//! - Validate: one call in flight per token, each run as its own spawned
//!   task. The task writes the cache, removes its in-flight entry through a
//!   drop guard (so even a panic leaves the map clean), then hands its answer
//!   to a channel. The map holds a shared copy of that channel's answer, and
//!   callers only await copies, so a caller that gives up leaves nothing
//!   half-run behind.
//! - Cache: Twitch's answer per token hash, for its `expires_in` but at most
//!   an hour (an hour when Twitch says 0); expired entries are pruned on each
//!   insert, and any 401 for the token drops its entry. Each hash carries an
//!   epoch that a drop bumps, so a validate already running never fills the
//!   entry back in.
//! - Failure memory: a refused fetch, a rejected token or a send answered 401
//!   is remembered for the account for 60 s, stamped with the forget counter
//!   and the account generation as they were before the fetch began; a stamp
//!   that moved voids the entry, and a fetch records nothing when something
//!   was forgotten while it ran. "Couldn't reach" is never remembered.
//! - The caller's budget bounds only the caller's own wait.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{FutureExt, Shared};

/// The longest Twitch's answer about a token is trusted.
pub const CACHE_MAX_MS: i64 = 3_600_000;
/// How long an account's failure is remembered.
pub const FAILURE_MS: i64 = 60_000;
/// The refusal text when Twitch rejected the token.
pub const REJECTED: &str = "Twitch rejected the sign-in";
/// The refusal text when the caller's time ran out.
pub const DEADLINE: &str = "Twitch didn't answer in time";
/// The answer when a validate task ended without one (it panicked or the
/// runtime stopped).
pub const STOPPED: &str = "The check with Twitch stopped";

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A token Twitch confirmed belongs to `user_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedToken {
    pub token: String,
    pub user_id: String,
    pub login: String,
}

/// Why no verified token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenRefusal {
    /// No token is stored for the account.
    NoToken(String),
    /// The fetch was refused or Twitch rejected the token (a 401).
    Refused(String),
    /// Twitch says the token is another user's (`actual` empty when it names none).
    WrongAccount { expected: String, actual: String },
    /// Twitch could not be asked, or the caller's time ran out: this event
    /// only, never remembered.
    Unreachable(String),
}

impl TokenRefusal {
    /// Remembered for the account for 60 s.
    pub fn remembered(&self) -> bool {
        matches!(self, TokenRefusal::NoToken(_) | TokenRefusal::Refused(_))
    }
}

/// Twitch's answer about one token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub user_id: String,
    pub login: String,
    /// Seconds the token has left (0 when Twitch does not say).
    pub expires_in_s: u64,
}

/// One validate call's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Validation {
    /// 200 with a user id.
    Valid(Identity),
    /// 200 without a user id: not a user's token.
    NoUser,
    /// 401.
    Rejected,
    /// A 5xx, a 429, a timeout or a network error.
    Unreachable(String),
}

/// The app's own token fetch for an account, sorted by kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetch {
    Token(String),
    /// No token stored, or its refresh was refused: sign in again.
    Refused(String),
    /// Twitch or the token store could not be reached: try the next event.
    Unreachable(String),
}

/// Everything the verifier reaches outside itself.
pub struct VerifyIo {
    /// The account's token through the app's own path.
    pub fetch: Box<dyn Fn(String) -> BoxFuture<Fetch> + Send + Sync>,
    /// Twitch's validate call for a token.
    pub validate: Box<dyn Fn(String) -> BoxFuture<Validation> + Send + Sync>,
    /// Runs a task on its own (real: `tokio::spawn`).
    pub spawn: Box<dyn Fn(BoxFuture<()>) + Send + Sync>,
    /// The token's hash (the cache key; a token is never kept as a key).
    pub token_hash: Box<dyn Fn(&str) -> String + Send + Sync>,
    /// Milliseconds on one steady clock.
    pub now_ms: Box<dyn Fn() -> i64 + Send + Sync>,
    /// The account generation (moves on every sign-in, switch and sign-out).
    pub generation: Box<dyn Fn() -> u64 + Send + Sync>,
}

struct Cached {
    user_id: String,
    login: String,
    until_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    forget: u64,
    generation: u64,
}

struct Failure {
    at_ms: i64,
    stamp: Stamp,
    refusal: TokenRefusal,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, Cached>,
    inflight: HashMap<String, InFlight>,
    /// The last in-flight sequence number handed out.
    seq: u64,
    /// Per hash: bumped by every drop, read by a validate before it writes.
    epochs: HashMap<String, u64>,
    failures: HashMap<String, Failure>,
    forget: u64,
}

/// One validate task's answer, shared by every caller, and the sequence
/// number its task removes it by.
struct InFlight {
    seq: u64,
    answer: Shared<BoxFuture<Validation>>,
}

struct Inner {
    io: VerifyIo,
    state: Mutex<State>,
}

/// Removes a validate task's in-flight entry when the task ends, however it
/// ends (an answer, a panic, a stopped runtime), and only while the entry is
/// still its own (a 401 drop may have detached it and a newer task taken
/// the hash).
struct EntryGuard {
    inner: Arc<Inner>,
    hash: String,
    seq: u64,
}

impl Drop for EntryGuard {
    fn drop(&mut self) {
        let mut state = self.inner.state();
        if state.inflight.get(&self.hash).is_some_and(|entry| entry.seq == self.seq) {
            state.inflight.remove(&self.hash);
        }
    }
}

impl Inner {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A validate task has its answer: write the cache, unless a 401 dropped
    /// the token since the task began (`epoch` moved).
    fn finish(&self, hash: &str, epoch: u64, answer: &Validation) {
        let done = (self.io.now_ms)();
        let mut state = self.state();
        if state.epochs.get(hash).copied().unwrap_or(0) != epoch {
            return;
        }
        match answer {
            Validation::Valid(identity) => {
                state.cache.retain(|_, c| done < c.until_ms);
                let keep = match identity.expires_in_s {
                    0 => CACHE_MAX_MS,
                    seconds => i64::try_from(seconds).unwrap_or(i64::MAX / 1000).saturating_mul(1000).min(CACHE_MAX_MS),
                };
                state.cache.insert(
                    hash.to_string(),
                    Cached { user_id: identity.user_id.clone(), login: identity.login.clone(), until_ms: done + keep },
                );
            }
            Validation::Rejected => {
                state.cache.remove(hash);
            }
            Validation::NoUser | Validation::Unreachable(_) => {}
        }
    }
}

pub struct TokenVerifier {
    inner: Arc<Inner>,
}

impl TokenVerifier {
    pub fn new(io: VerifyIo) -> TokenVerifier {
        TokenVerifier { inner: Arc::new(Inner { io, state: Mutex::new(State::default()) }) }
    }

    fn io(&self) -> &VerifyIo {
        &self.inner.io
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state()
    }

    /// The stamps a fetch starting now carries.
    fn stamp(&self) -> Stamp {
        let forget = self.state().forget;
        Stamp { forget, generation: (self.io().generation)() }
    }

    /// The account's remembered refusal, while it still counts.
    pub fn remembered_failure(&self, account_id: &str) -> Option<TokenRefusal> {
        let now = (self.io().now_ms)();
        let generation = (self.io().generation)();
        let state = self.state();
        let current = Stamp { forget: state.forget, generation };
        state
            .failures
            .get(account_id)
            .filter(|f| now - f.at_ms < FAILURE_MS && f.stamp == current)
            .map(|f| f.refusal.clone())
    }

    fn record(&self, account_id: &str, refusal: TokenRefusal, stamp: Stamp) {
        let now = (self.io().now_ms)();
        let mut state = self.state();
        if state.forget != stamp.forget {
            return;
        }
        state.failures.insert(account_id.to_string(), Failure { at_ms: now, stamp, refusal });
    }

    /// A Helix send as `account_id` was answered 401: remember it for 60 s,
    /// so the next fires make no validate and no send.
    pub fn record_send_401(&self, account_id: &str) {
        let stamp = self.stamp();
        self.record(account_id, TokenRefusal::Refused(REJECTED.to_string()), stamp);
    }

    /// The token to act as `account_id`, once Twitch confirmed it is that
    /// account's. `budget` bounds this caller's wait only (the job's
    /// remaining age budget, 5 s for a release attempt, 15 s for a reminder).
    pub async fn verify(&self, account_id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal> {
        match tokio::time::timeout(budget, self.verify_now(account_id)).await {
            Ok(result) => result,
            Err(_) => Err(TokenRefusal::Unreachable(DEADLINE.to_string())),
        }
    }

    async fn verify_now(&self, account_id: &str) -> Result<VerifiedToken, TokenRefusal> {
        if let Some(refusal) = self.remembered_failure(account_id) {
            return Err(refusal);
        }
        let stamp = self.stamp();
        let token = match (self.io().fetch)(account_id.to_string()).await {
            Fetch::Token(token) => token,
            Fetch::Refused(why) => {
                let refusal = TokenRefusal::Refused(why);
                self.record(account_id, refusal.clone(), stamp);
                return Err(refusal);
            }
            Fetch::Unreachable(why) => return Err(TokenRefusal::Unreachable(why)),
        };
        match self.validate(&token).await {
            Validation::Valid(identity) if identity.user_id == account_id => {
                Ok(VerifiedToken { token, user_id: identity.user_id, login: identity.login })
            }
            Validation::Valid(identity) => {
                Err(TokenRefusal::WrongAccount { expected: account_id.to_string(), actual: identity.user_id })
            }
            Validation::NoUser => Err(TokenRefusal::WrongAccount { expected: account_id.to_string(), actual: String::new() }),
            Validation::Rejected => {
                let refusal = TokenRefusal::Refused(REJECTED.to_string());
                self.record(account_id, refusal.clone(), stamp);
                Err(refusal)
            }
            Validation::Unreachable(why) => Err(TokenRefusal::Unreachable(why)),
        }
    }

    /// Twitch's answer about a token: from the cache, or the one validate
    /// task running for it (started now when none is).
    pub async fn validate(&self, token: &str) -> Validation {
        let hash = (self.io().token_hash)(token);
        let now = (self.io().now_ms)();
        let running = {
            let mut state = self.state();
            if let Some(hit) = state.cache.get(&hash).filter(|c| now < c.until_ms) {
                return Validation::Valid(Identity {
                    user_id: hit.user_id.clone(),
                    login: hit.login.clone(),
                    expires_in_s: u64::try_from((hit.until_ms - now) / 1000).unwrap_or(0),
                });
            }
            let existing = state.inflight.get(&hash).map(|entry| entry.answer.clone());
            if let Some(running) = existing {
                running
            } else {
                // The entry is in the map before the task exists, so the
                // task's guard always finds it to remove; the task is spawned
                // after the lock is released, so a spawner that runs the task
                // at once, or drops it, never waits on this lock.
                let (send, answer) = oneshot::channel::<Validation>();
                let shared: BoxFuture<Validation> =
                    Box::pin(async move { answer.await.unwrap_or_else(|_| Validation::Unreachable(STOPPED.to_string())) });
                let shared = shared.shared();
                state.seq += 1;
                let seq = state.seq;
                state.inflight.insert(hash.clone(), InFlight { seq, answer: shared.clone() });
                let epoch = state.epochs.get(&hash).copied().unwrap_or(0);
                drop(state);
                // The guard exists before anything that can panic or be
                // dropped unrun (making the validate future, the spawn), and
                // it moves into the task: however the task ends, or never
                // starts, the entry leaves the map (sequence-checked).
                let guard = EntryGuard { inner: Arc::clone(&self.inner), hash, seq };
                let call = (self.io().validate)(token.to_string());
                (self.io().spawn)(Box::pin(async move {
                    let answer = call.await;
                    guard.inner.finish(&guard.hash, epoch, &answer);
                    drop(guard);
                    let _ = send.send(answer);
                }));
                return shared.await;
            }
        };
        running.await
    }

    /// Twitch answered 401 for the token with this hash (a send, a claim, a
    /// release or a validate): forget what it said about it, detach a
    /// validate already running (later callers start a new one), and keep
    /// that task from writing its answer back.
    pub fn drop_token(&self, hash: &str) {
        let mut state = self.state();
        state.cache.remove(hash);
        state.inflight.remove(hash);
        *state.epochs.entry(hash.to_string()).or_insert(0) += 1;
    }

    /// [`Self::drop_token`] for a token.
    pub fn drop_token_of(&self, token: &str) {
        let hash = (self.io().token_hash)(token);
        self.drop_token(&hash);
    }

    /// The accounts changed (a save, a new identity): every remembered
    /// failure is forgotten, and a fetch still running records none.
    pub fn forget_failures(&self) {
        let mut state = self.state();
        state.forget += 1;
        state.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::oneshot;
    use futures::future::join_all;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    const ME: &str = "100";
    const OTHER: &str = "999";
    /// A budget no test reaches.
    const B: Duration = Duration::from_secs(5);

    /// Pending once, then ready.
    #[derive(Default)]
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                Poll::Ready(())
            } else {
                self.0 = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    #[derive(Default)]
    struct Fake {
        /// The fetch's answers in turn; the last one repeats.
        fetches: Mutex<Vec<Fetch>>,
        fetch_calls: AtomicUsize,
        /// The fetch never answers.
        fetch_hangs: AtomicBool,
        /// The next fetch parks until this is answered.
        fetch_gate: Mutex<Option<oneshot::Receiver<()>>>,
        /// The fetch takes this lock before answering.
        fetch_lock: Mutex<Option<Arc<tokio::sync::Mutex<()>>>>,
        /// What Twitch says about each token (unknown tokens are rejected).
        who: Mutex<HashMap<String, Validation>>,
        validate_calls: AtomicUsize,
        /// The next validate parks until this is answered.
        validate_gate: Mutex<Option<oneshot::Receiver<()>>>,
        /// Every validate answers after this many milliseconds (paused time).
        validate_delay_ms: AtomicU64,
        now: AtomicI64,
        generation: AtomicU64,
    }

    impl Fake {
        fn fetches(&self, answers: Vec<Fetch>) {
            *self.fetches.lock().unwrap() = answers;
            self.fetch_calls.store(0, Ordering::SeqCst);
        }
        fn says(&self, token: &str, answer: Validation) {
            self.who.lock().unwrap().insert(token.to_string(), answer);
        }
        fn count(n: &AtomicUsize) -> usize {
            n.load(Ordering::SeqCst)
        }
        fn advance(&self, ms: i64) {
            self.now.fetch_add(ms, Ordering::SeqCst);
        }
    }

    fn valid(user_id: &str) -> Validation {
        valid_for(user_id, 3_600)
    }

    fn valid_for(user_id: &str, expires_in_s: u64) -> Validation {
        Validation::Valid(Identity { user_id: user_id.into(), login: format!("login{user_id}"), expires_in_s })
    }

    fn token(t: &str) -> Fetch {
        Fetch::Token(t.to_string())
    }

    /// A verifier over fakes whose validate tasks run on the test runtime.
    fn verifier() -> (Arc<Fake>, TokenVerifier) {
        build(Box::new(|task: BoxFuture<()>| {
            tokio::spawn(task);
        }))
    }

    /// The same, with a spawner that runs each task to the end before it
    /// returns.
    fn verifier_inline() -> (Arc<Fake>, TokenVerifier) {
        build(Box::new(|task: BoxFuture<()>| futures::executor::block_on(task)))
    }

    /// The same, with a spawner that drops each task without running it.
    fn verifier_dropping() -> (Arc<Fake>, TokenVerifier) {
        build(Box::new(|task: BoxFuture<()>| drop(task)))
    }

    fn build(spawn: Box<dyn Fn(BoxFuture<()>) + Send + Sync>) -> (Arc<Fake>, TokenVerifier) {
        let fake = Arc::new(Fake::default());
        let (f1, f2, f3, f4) = (fake.clone(), fake.clone(), fake.clone(), fake.clone());
        let io = VerifyIo {
            fetch: Box::new(move |_account: String| -> BoxFuture<Fetch> {
                let f = f1.clone();
                let n = f.fetch_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    YieldOnce::default().await;
                    if f.fetch_hangs.load(Ordering::SeqCst) {
                        futures::future::pending::<()>().await;
                    }
                    let gate = f.fetch_gate.lock().unwrap().take();
                    if let Some(gate) = gate {
                        let _ = gate.await;
                    }
                    let held = f.fetch_lock.lock().unwrap().clone();
                    if let Some(held) = held {
                        let _guard = held.lock().await;
                    }
                    let answers = f.fetches.lock().unwrap().clone();
                    let answer = answers.get(n.min(answers.len().saturating_sub(1))).cloned();
                    answer.unwrap_or(Fetch::Refused("No stored token".into()))
                })
            }),
            validate: Box::new(move |token: String| -> BoxFuture<Validation> {
                let f = f2.clone();
                f.validate_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    let gate = f.validate_gate.lock().unwrap().take();
                    if let Some(gate) = gate {
                        let _ = gate.await;
                    }
                    let delay = f.validate_delay_ms.load(Ordering::SeqCst);
                    if delay > 0 {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    } else {
                        YieldOnce::default().await;
                    }
                    if token == "PANIC" {
                        panic!("the validate task panics");
                    }
                    let answer = f.who.lock().unwrap().get(&token).cloned();
                    answer.unwrap_or(Validation::Rejected)
                })
            }),
            spawn,
            token_hash: Box::new(|token: &str| format!("hash-{token}")),
            now_ms: Box::new(move || f3.now.load(Ordering::SeqCst)),
            generation: Box::new(move || f4.generation.load(Ordering::SeqCst)),
        };
        (fake, TokenVerifier::new(io))
    }

    fn inflight(v: &TokenVerifier) -> usize {
        v.state().inflight.len()
    }

    fn cached(v: &TokenVerifier, token: &str) -> bool {
        v.state().cache.contains_key(&format!("hash-{token}"))
    }

    #[tokio::test(start_paused = true)]
    async fn a_token_twitch_says_is_someone_elses_is_refused() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(OTHER));
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::WrongAccount { expected: ME.into(), actual: OTHER.into() }));
        assert_eq!(v.remembered_failure(ME), None);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 2);
        // A token that names no user is never anyone's.
        fake.says("T1", Validation::NoUser);
        v.drop_token_of("T1");
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::WrongAccount { expected: ME.into(), actual: String::new() }));
        // The account's own token passes.
        fake.says("T1", valid(ME));
        assert_eq!(
            v.verify(ME, B).await,
            Ok(VerifiedToken { token: "T1".into(), user_id: ME.into(), login: format!("login{ME}") })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_fetch_costs_one_fetch_a_minute() {
        let (fake, v) = verifier();
        fake.fetches(vec![Fetch::Refused("refresh refused".into())]);
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Refused("refresh refused".into())));
        fake.advance(59_000);
        assert!(v.verify(ME, B).await.is_err());
        assert_eq!(Fake::count(&fake.fetch_calls), 1);
        // After 60 s it fetches again.
        fake.advance(1_000);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 2);
        // After forget_failures().
        v.forget_failures();
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 3);
        // After the account generation moves with no save.
        fake.generation.fetch_add(1, Ordering::SeqCst);
        assert_eq!(v.remembered_failure(ME), None);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 4);
    }

    #[tokio::test(start_paused = true)]
    async fn a_fetch_records_its_failure_with_the_stamps_it_began_under() {
        // Began before forget_failures() and failed after it: nothing recorded.
        let (fake, v) = verifier();
        fake.fetches(vec![Fetch::Refused("refresh refused".into())]);
        let mut running = Box::pin(v.verify(ME, B));
        assert!(futures::poll!(running.as_mut()).is_pending());
        v.forget_failures();
        assert!(running.await.is_err());
        assert_eq!(v.remembered_failure(ME), None);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 2);
        // Parked on a oneshot while the generation moves: the failure carries
        // the old generation, so the next event fetches.
        let (fake, v) = verifier();
        fake.fetches(vec![Fetch::Refused("refresh refused".into())]);
        let (release, gate) = oneshot::channel::<()>();
        *fake.fetch_gate.lock().unwrap() = Some(gate);
        let mut running = Box::pin(v.verify(ME, B));
        for _ in 0..2 {
            assert!(futures::poll!(running.as_mut()).is_pending());
        }
        fake.generation.fetch_add(1, Ordering::SeqCst);
        release.send(()).unwrap();
        assert!(running.await.is_err());
        assert_eq!(v.remembered_failure(ME), None);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.fetch_calls), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreachable_fetch_skips_this_event_only() {
        let (fake, v) = verifier();
        fake.fetches(vec![Fetch::Unreachable("503".into()), token("T1")]);
        fake.says("T1", valid(ME));
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Unreachable("503".into())));
        assert_eq!(v.remembered_failure(ME), None);
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.fetch_calls), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_token_that_changes_hands_is_never_used_for_the_wrong_account() {
        // The fetch answers T1 (the owner's) first and T2 (another user's) after.
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1"), token("T2")]);
        fake.says("T1", valid(ME));
        fake.says("T2", valid(OTHER));
        assert_eq!(v.verify(ME, B).await.unwrap().token, "T1");
        assert!(v.verify(ME, B).await.is_err(), "T2 is never handed out as {ME}");
        // Control: a fetch that always answers T1 gives T1 every time.
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        assert_eq!(v.verify(ME, B).await.unwrap().token, "T1");
        assert_eq!(v.verify(ME, B).await.unwrap().token, "T1");
    }

    #[tokio::test(start_paused = true)]
    async fn one_validate_per_distinct_token() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        assert!(v.verify(ME, B).await.is_ok());
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.validate_calls), 1);
        // A refreshed token: one more.
        fake.fetches(vec![token("T2")]);
        fake.says("T2", valid(ME));
        assert_eq!(v.verify(ME, B).await.unwrap().token, "T2");
        assert_eq!(Fake::count(&fake.validate_calls), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn eight_cold_callers_share_one_validate_held_open() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        let (release, gate) = oneshot::channel::<()>();
        *fake.validate_gate.lock().unwrap() = Some(gate);
        let mut callers: Vec<_> = (0..8).map(|_| Box::pin(v.verify(ME, B))).collect();
        // Past the fetch, then waiting on the one validate.
        for _ in 0..3 {
            for caller in callers.iter_mut() {
                assert!(futures::poll!(caller.as_mut()).is_pending());
            }
        }
        assert_eq!(Fake::count(&fake.validate_calls), 1);
        release.send(()).unwrap();
        let answers = join_all(callers).await;
        assert!(answers.iter().all(|a| a.as_ref().map(|t| t.token.as_str()) == Ok("T1")));
        assert_eq!(Fake::count(&fake.validate_calls), 1);
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_that_times_out_leaves_nothing_behind() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        fake.validate_delay_ms.store(200, Ordering::SeqCst);
        let answers = join_all((0..3).map(|_| v.verify(ME, Duration::from_millis(50)))).await;
        assert!(answers.iter().all(|a| *a == Err(TokenRefusal::Unreachable(DEADLINE.into()))));
        assert_eq!(Fake::count(&fake.validate_calls), 1);
        tokio::time::sleep(Duration::from_millis(250)).await;
        // The spawned task finished on its own: cached, out of the map.
        assert_eq!(inflight(&v), 0);
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.validate_calls), 1);
        // After an unreachable answer, the next ask is a new call.
        fake.validate_delay_ms.store(0, Ordering::SeqCst);
        fake.says("U", Validation::Unreachable("timeout".into()));
        assert_eq!(v.validate("U").await, Validation::Unreachable("timeout".into()));
        assert_eq!(v.validate("U").await, Validation::Unreachable("timeout".into()));
        assert_eq!(Fake::count(&fake.validate_calls), 3);
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_spawner_that_runs_the_task_at_once_leaves_the_map_empty() {
        let (fake, v) = verifier_inline();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(inflight(&v), 0);
        assert!(cached(&v, "T1"));
        // Nothing stranded: an unreachable answer is asked again, and a 401
        // drop makes the next verify a new call.
        fake.says("U", Validation::Unreachable("timeout".into()));
        assert_eq!(v.validate("U").await, Validation::Unreachable("timeout".into()));
        assert_eq!(v.validate("U").await, Validation::Unreachable("timeout".into()));
        v.drop_token_of("T1");
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.validate_calls), 4);
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_task_dropped_without_running_leaves_the_map_empty() {
        let (fake, v) = verifier_dropping();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        // The verify in flight when its task is dropped gets the canceled
        // answer as Unreachable (no panic), and nothing is remembered.
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Unreachable(STOPPED.into())));
        assert_eq!(inflight(&v), 0);
        assert_eq!(v.remembered_failure(ME), None);
        // The next verify makes a new validate, not a wait on a stranded one.
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Unreachable(STOPPED.into())));
        assert_eq!(Fake::count(&fake.validate_calls), 2);
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn no_stored_future_holds_a_lock() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        *fake.fetch_lock.lock().unwrap() = Some(lock.clone());
        let guard = lock.lock().await;
        assert_eq!(v.verify(ME, Duration::from_millis(10)).await, Err(TokenRefusal::Unreachable(DEADLINE.into())));
        drop(guard);
        assert!(tokio::time::timeout(Duration::from_millis(100), lock.lock()).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_fetch_that_never_answers_ends_inside_the_budget() {
        let (fake, v) = verifier();
        fake.fetch_hangs.store(true, Ordering::SeqCst);
        let started = tokio::time::Instant::now();
        assert_eq!(v.verify(ME, Duration::from_millis(50)).await, Err(TokenRefusal::Unreachable(DEADLINE.into())));
        assert!(started.elapsed() <= Duration::from_millis(60));
        assert_eq!(v.remembered_failure(ME), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_cache_lasts_at_most_an_hour_and_prunes_on_insert() {
        let (fake, v) = verifier();
        fake.says("long", valid_for(ME, 20_000));
        fake.says("zero", valid_for(ME, 0));
        fake.says("short", valid_for(ME, 100));
        for t in ["long", "zero", "short"] {
            v.validate(t).await;
        }
        assert_eq!(Fake::count(&fake.validate_calls), 3);
        fake.advance(101_000);
        v.validate("short").await;
        assert_eq!(Fake::count(&fake.validate_calls), 4);
        fake.advance(CACHE_MAX_MS - 101_000 - 1);
        v.validate("long").await;
        v.validate("zero").await;
        assert_eq!(Fake::count(&fake.validate_calls), 4);
        fake.advance(1);
        v.validate("long").await;
        v.validate("zero").await;
        assert_eq!(Fake::count(&fake.validate_calls), 6);
        assert!(v.state().cache.len() <= 3);
        fake.advance(2 * CACHE_MAX_MS);
        fake.says("fresh", valid_for(ME, 60));
        v.validate("fresh").await;
        assert_eq!(v.state().cache.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_401_anywhere_drops_the_tokens_entry() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        assert!(v.verify(ME, B).await.is_ok());
        // A Helix send, claim or release answered 401.
        v.drop_token_of("T1");
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.validate_calls), 2);
        v.drop_token("hash-T1");
        assert!(v.verify(ME, B).await.is_ok());
        assert_eq!(Fake::count(&fake.validate_calls), 3);
        // A validate 401 drops it too.
        fake.says("T1", Validation::Rejected);
        v.drop_token_of("T1");
        let _ = v.validate("T1").await;
        assert!(!cached(&v, "T1"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_401_drop_during_a_running_validate_starts_a_new_one_and_is_not_refilled() {
        let (fake, v) = verifier();
        fake.says("T1", valid(ME));
        let (release, gate) = oneshot::channel::<()>();
        *fake.validate_gate.lock().unwrap() = Some(gate);
        let mut first = Box::pin(v.validate("T1"));
        assert!(futures::poll!(first.as_mut()).is_pending());
        // Let the first task start and park on its gate.
        tokio::task::yield_now().await;
        // A send answered 401 while that validate was still running: it is
        // detached, and a later caller starts a new validate.
        v.drop_token_of("T1");
        assert_eq!(inflight(&v), 0);
        assert_eq!(v.validate("T1").await, valid(ME));
        assert_eq!(Fake::count(&fake.validate_calls), 2);
        assert!(cached(&v, "T1"));
        // Another 401: whatever the old task writes now would be its own.
        v.drop_token_of("T1");
        release.send(()).unwrap();
        assert_eq!(first.await, valid(ME));
        assert!(!cached(&v, "T1"));
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_validate_task_that_panics_leaves_the_map_empty() {
        let (fake, v) = verifier();
        assert_eq!(v.validate("PANIC").await, Validation::Unreachable(STOPPED.into()));
        assert_eq!(inflight(&v), 0);
        // The next ask is a new call, not the stranded one.
        assert_eq!(v.validate("PANIC").await, Validation::Unreachable(STOPPED.into()));
        assert_eq!(Fake::count(&fake.validate_calls), 2);
        assert_eq!(inflight(&v), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_send_401_is_remembered_for_a_minute() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", valid(ME));
        assert!(v.verify(ME, B).await.is_ok());
        v.drop_token_of("T1");
        v.record_send_401(ME);
        let (fetches, validates) = (Fake::count(&fake.fetch_calls), Fake::count(&fake.validate_calls));
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Refused(REJECTED.into())));
        assert_eq!((Fake::count(&fake.fetch_calls), Fake::count(&fake.validate_calls)), (fetches, validates));
        fake.advance(FAILURE_MS);
        assert!(v.verify(ME, B).await.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn a_validate_401_is_remembered_and_unreachable_is_not() {
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", Validation::Rejected);
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Refused(REJECTED.into())));
        let (fetches, validates) = (Fake::count(&fake.fetch_calls), Fake::count(&fake.validate_calls));
        fake.advance(59_000);
        assert!(v.verify(ME, B).await.is_err());
        assert_eq!((Fake::count(&fake.fetch_calls), Fake::count(&fake.validate_calls)), (fetches, validates));
        // 5xx, 429 and timeouts reach the verifier as Unreachable: not remembered.
        let (fake, v) = verifier();
        fake.fetches(vec![token("T1")]);
        fake.says("T1", Validation::Unreachable("429".into()));
        assert_eq!(v.verify(ME, B).await, Err(TokenRefusal::Unreachable("429".into())));
        assert_eq!(v.remembered_failure(ME), None);
        let _ = v.verify(ME, B).await;
        assert_eq!(Fake::count(&fake.validate_calls), 2);
    }
}
