//! A keyed cache whose misses are read once, in a task that outlives its callers.
//!
//! A caller asks for a key with a time to live and a budget. A value younger than
//! the time to live answers at once. Otherwise the caller joins the read already
//! running for that key, or starts one, and waits at most the budget. A read that
//! outlasts the budget goes on in its own task and fills the cache when it ends,
//! so the next caller finds it; while it runs, or when it fails, the caller gets
//! the older value if there is one.
//!
//! Every stored value takes the next number from one counter. A read writes only
//! if its key still holds the number it started with, and `clear` ends the claim
//! of every read that began before it, so a value the owner just wrote (`prime`)
//! is never overwritten by a read that began earlier. A failed read is never
//! stored.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// How one `lookup` ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup<V> {
    /// Held within its time to live, or just read (or joined).
    Fresh(V),
    /// Older than its time to live: the read did not finish in the budget, or failed.
    Stale(V),
    /// The read failed and nothing is kept for the key.
    Failed(String),
    /// The budget ran out with nothing kept; the read goes on and fills the cache.
    Pending,
}

impl<V> Lookup<V> {
    /// The value, fresh or stale.
    pub fn value(self) -> Option<V> {
        match self {
            Lookup::Fresh(v) | Lookup::Stale(v) => Some(v),
            Lookup::Failed(_) | Lookup::Pending => None,
        }
    }
}

/// Where one running read stands.
#[derive(Clone)]
enum Progress<V> {
    Running,
    Done(V),
    Failed(String),
    /// The read finished but the key had been written (or cleared) since it began.
    Superseded,
}

struct Entry<V> {
    seq: u64,
    at: Instant,
    value: V,
}

struct Flight<V> {
    id: u64,
    rx: watch::Receiver<Progress<V>>,
}

struct Inner<V> {
    /// The last number handed out; every store takes the next one.
    seq: u64,
    /// Bumped by `clear`: a read that began before it never writes.
    epoch: u64,
    next_flight: u64,
    entries: HashMap<String, Entry<V>>,
    flights: HashMap<String, Flight<V>>,
}

impl<V> Inner<V> {
    fn store(&mut self, key: &str, value: V, cap: usize) {
        self.seq += 1;
        if !self.entries.contains_key(key) && self.entries.len() >= cap {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.seq)
                .map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(
            key.to_string(),
            Entry {
                seq: self.seq,
                at: Instant::now(),
                value,
            },
        );
    }
}

fn lock<V>(inner: &Mutex<Inner<V>>) -> MutexGuard<'_, Inner<V>> {
    inner.lock().unwrap_or_else(|e| e.into_inner())
}

/// Takes a read's flight off the map however its task ends (a panic included),
/// so a dead read can never be joined.
struct Landing<V> {
    inner: Arc<Mutex<Inner<V>>>,
    key: String,
    id: u64,
}

impl<V> Drop for Landing<V> {
    fn drop(&mut self) {
        let mut g = lock(&self.inner);
        if g.flights.get(&self.key).is_some_and(|f| f.id == self.id) {
            g.flights.remove(&self.key);
        }
    }
}

pub struct SharedLookup<V> {
    cap: usize,
    inner: Arc<Mutex<Inner<V>>>,
}

impl<V: Clone + Send + Sync + 'static> SharedLookup<V> {
    /// A cache holding at most `cap` keys; the oldest write is dropped first.
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            inner: Arc::new(Mutex::new(Inner {
                seq: 0,
                epoch: 0,
                next_flight: 0,
                entries: HashMap::new(),
                flights: HashMap::new(),
            })),
        }
    }

    /// The value for `key`: fresh within `ttl`, else the result of the one read
    /// `make` starts (or the read already running), waited for at most `budget`.
    /// `make` only builds the future (it runs while the cache is locked); the
    /// future itself runs in its own task. `None` is a budget miss or a failure
    /// with nothing kept.
    pub async fn get<Fut>(
        &self,
        key: &str,
        ttl: Duration,
        budget: Duration,
        make: impl FnOnce() -> Fut,
    ) -> Option<V>
    where
        Fut: Future<Output = Result<V, String>> + Send + 'static,
    {
        self.lookup(key, ttl, budget, make).await.value()
    }

    /// `get`, saying how it ended.
    pub async fn lookup<Fut>(
        &self,
        key: &str,
        ttl: Duration,
        budget: Duration,
        make: impl FnOnce() -> Fut,
    ) -> Lookup<V>
    where
        Fut: Future<Output = Result<V, String>> + Send + 'static,
    {
        let (stale, mut rx) = {
            let mut guard = lock(&self.inner);
            let g: &mut Inner<V> = &mut guard;
            if let Some(e) = g.entries.get(key) {
                if e.at.elapsed() < ttl {
                    return Lookup::Fresh(e.value.clone());
                }
            }
            let stale = g.entries.get(key).map(|e| e.value.clone());
            let joined = g.flights.get(key).map(|f| f.rx.clone());
            let rx = match joined {
                Some(rx) => rx,
                None => {
                    g.next_flight += 1;
                    let id = g.next_flight;
                    let started = g.entries.get(key).map_or(0, |e| e.seq);
                    let epoch = g.epoch;
                    let (tx, rx) = watch::channel(Progress::Running);
                    g.flights
                        .insert(key.to_string(), Flight { id, rx: rx.clone() });
                    let fut = make();
                    let landing = Landing {
                        inner: self.inner.clone(),
                        key: key.to_string(),
                        id,
                    };
                    let inner = self.inner.clone();
                    let cap = self.cap;
                    let key = key.to_string();
                    tauri::async_runtime::spawn(async move {
                        let _landing = landing;
                        let result = fut.await;
                        let progress = {
                            let mut guard = lock(&inner);
                            let g: &mut Inner<V> = &mut guard;
                            if g.flights.get(&key).is_some_and(|f| f.id == id) {
                                g.flights.remove(&key);
                            }
                            match result {
                                Err(e) => Progress::Failed(e),
                                Ok(value) => {
                                    let now = g.entries.get(&key).map_or(0, |e| e.seq);
                                    if now == started && g.epoch == epoch {
                                        g.store(&key, value.clone(), cap);
                                        Progress::Done(value)
                                    } else {
                                        Progress::Superseded
                                    }
                                }
                            }
                        };
                        let _ = tx.send(progress);
                    });
                    rx
                }
            };
            (stale, rx)
        };
        let waited = tokio::time::timeout(budget, async {
            loop {
                let seen = rx.borrow_and_update().clone();
                match seen {
                    Progress::Running => {
                        if rx.changed().await.is_err() {
                            return Progress::Failed(
                                "the read ended without an answer".to_string(),
                            );
                        }
                    }
                    done => return done,
                }
            }
        })
        .await;
        match waited {
            Ok(Progress::Done(v)) => Lookup::Fresh(v),
            Ok(Progress::Failed(e)) => stale.map_or(Lookup::Failed(e), Lookup::Stale),
            Ok(Progress::Superseded) => self.peek(key).map_or(Lookup::Pending, Lookup::Fresh),
            Ok(Progress::Running) | Err(_) => stale.map_or(Lookup::Pending, Lookup::Stale),
        }
    }

    /// The value held for `key`, whatever its age. Asks nothing.
    pub fn peek(&self, key: &str) -> Option<V> {
        lock(&self.inner).entries.get(key).map(|e| e.value.clone())
    }

    /// The number of the write that holds `key` (0 when nothing does). A caller
    /// that read the value can tell later whether it was written over.
    pub fn stamp(&self, key: &str) -> u64 {
        lock(&self.inner).entries.get(key).map_or(0, |e| e.seq)
    }

    /// Stores a value the owner just wrote. Any read that began earlier loses.
    pub fn prime(&self, key: &str, value: V) {
        lock(&self.inner).store(key, value, self.cap);
    }

    /// Forgets every value and ends the claim of every read now running.
    pub fn clear(&self) {
        let mut guard = lock(&self.inner);
        guard.entries.clear();
        guard.flights.clear();
        guard.epoch += 1;
        guard.seq += 1;
    }

    pub fn len(&self) -> usize {
        lock(&self.inner).entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    const LONG: Duration = Duration::from_secs(10);

    /// Polls `probe` until it answers (up to two seconds).
    async fn eventually<T>(mut probe: impl FnMut() -> Option<T>) -> T {
        for _ in 0..200 {
            if let Some(v) = probe() {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the awaited state never came");
    }

    fn flights<V>(cache: &SharedLookup<V>) -> usize {
        lock(&cache.inner).flights.len()
    }

    #[tokio::test]
    async fn a_miss_reads_once_and_a_fresh_hit_never_reads() {
        let cache = SharedLookup::<u32>::new(8);
        let reads = Arc::new(AtomicUsize::new(0));
        let make = |n: u32| {
            let reads = reads.clone();
            move || async move {
                reads.fetch_add(1, Ordering::SeqCst);
                Ok::<u32, String>(n)
            }
        };
        assert_eq!(
            cache.lookup("k", LONG, LONG, make(7)).await,
            Lookup::Fresh(7)
        );
        assert_eq!(
            cache.lookup("k", LONG, LONG, make(8)).await,
            Lookup::Fresh(7)
        );
        assert_eq!(cache.get("k", LONG, LONG, make(9)).await, Some(7));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn two_concurrent_misses_share_one_read() {
        let cache = SharedLookup::<u32>::new(8);
        let calls = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = oneshot::channel::<u32>();
        let first = {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move { rx.await.map_err(|e| e.to_string()) }
            }
        };
        let second = {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                async move { Ok::<u32, String>(99) }
            }
        };
        let release = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = tx.send(5);
        };
        let (a, b, _) = tokio::join!(
            cache.lookup("k", LONG, LONG, first),
            cache.lookup("k", LONG, LONG, second),
            release
        );
        assert_eq!((a, b), (Lookup::Fresh(5), Lookup::Fresh(5)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the second miss joined the first read"
        );
    }

    #[tokio::test]
    async fn a_budget_miss_returns_the_stale_value_and_the_late_read_still_fills_the_cache() {
        let cache = SharedLookup::<u32>::new(8);
        cache.prime("k", 1);
        let (tx, rx) = oneshot::channel::<u32>();
        let slow = move || async move { rx.await.map_err(|e| e.to_string()) };
        let got = cache
            .lookup("k", Duration::ZERO, Duration::from_millis(20), slow)
            .await;
        assert_eq!(got, Lookup::Stale(1));
        let _ = tx.send(2);
        eventually(|| (flights(&cache) == 0).then_some(())).await;
        assert_eq!(
            cache.peek("k"),
            Some(2),
            "the detached read wrote when it ended"
        );
    }

    #[tokio::test]
    async fn a_read_that_never_finishes_ends_at_the_budget() {
        let cache = SharedLookup::<u32>::new(8);
        let ended = tokio::time::timeout(
            Duration::from_secs(5),
            cache.lookup("k", LONG, Duration::from_millis(50), || {
                std::future::pending::<Result<u32, String>>()
            }),
        )
        .await;
        assert_eq!(
            ended.expect("the budget did not end the wait"),
            Lookup::Pending
        );
        assert_eq!(cache.len(), 0);
        assert_eq!(
            flights(&cache),
            1,
            "the read stays in flight for the next caller to join"
        );
    }

    #[tokio::test]
    async fn a_late_read_never_overwrites_a_prime() {
        let cache = SharedLookup::<u32>::new(8);
        let (tx, rx) = oneshot::channel::<u32>();
        let first = cache
            .lookup("k", LONG, Duration::from_millis(20), move || async move {
                rx.await.map_err(|e| e.to_string())
            })
            .await;
        assert_eq!(first, Lookup::Pending);
        cache.prime("k", 9);
        let _ = tx.send(5);
        eventually(|| (flights(&cache) == 0).then_some(())).await;
        assert_eq!(cache.peek("k"), Some(9));
    }

    #[tokio::test]
    async fn a_joiner_of_a_superseded_read_gets_the_prime() {
        let cache = SharedLookup::<u32>::new(8);
        let (tx, rx) = oneshot::channel::<u32>();
        let waiting = cache.lookup("k", LONG, LONG, move || async move {
            rx.await.map_err(|e| e.to_string())
        });
        let write = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cache.prime("k", 9);
            let _ = tx.send(5);
        };
        let (got, _) = tokio::join!(waiting, write);
        assert_eq!(got, Lookup::Fresh(9));
    }

    #[tokio::test]
    async fn errors_are_never_cached() {
        let cache = SharedLookup::<u32>::new(8);
        let first = cache
            .lookup("k", LONG, LONG, || async {
                Err::<u32, String>("down".into())
            })
            .await;
        assert_eq!(first, Lookup::Failed("down".into()));
        assert_eq!(cache.peek("k"), None);
        let second = cache
            .lookup("k", LONG, LONG, || async { Ok::<u32, String>(3) })
            .await;
        assert_eq!(second, Lookup::Fresh(3));
    }

    #[tokio::test]
    async fn a_failed_read_keeps_the_older_value() {
        let cache = SharedLookup::<u32>::new(8);
        cache.prime("k", 1);
        let got = cache
            .lookup("k", Duration::ZERO, LONG, || async {
                Err::<u32, String>("down".into())
            })
            .await;
        assert_eq!(got, Lookup::Stale(1));
        assert_eq!(cache.peek("k"), Some(1));
    }

    #[test]
    fn capacity_is_bounded_and_the_oldest_write_goes_first() {
        let cache = SharedLookup::<u32>::new(2);
        cache.prime("a", 1);
        cache.prime("b", 2);
        cache.prime("a", 10);
        cache.prime("c", 3);
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.peek("b"), None, "b was the oldest write");
        assert_eq!((cache.peek("a"), cache.peek("c")), (Some(10), Some(3)));
        assert!(cache.stamp("c") > cache.stamp("a"));
        assert_eq!(cache.stamp("gone"), 0);
    }

    #[tokio::test]
    async fn clear_forgets_values_and_ends_the_claim_of_reads_that_began_before_it() {
        let cache = SharedLookup::<u32>::new(8);
        cache.prime("k", 1);
        let (tx, rx) = oneshot::channel::<u32>();
        let began = cache
            .lookup("j", LONG, Duration::from_millis(20), move || async move {
                rx.await.map_err(|e| e.to_string())
            })
            .await;
        assert_eq!(began, Lookup::Pending);
        cache.clear();
        assert_eq!((cache.peek("k"), cache.len()), (None, 0));
        let _ = tx.send(4);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            cache.peek("j"),
            None,
            "a read that began before clear never writes"
        );
    }
}
