//! What a category's tag filter offers, in one answer for the desktop search
//! box and both phones' chip row and tag sheet.
//!
//! Rust knows each category's own tags (from its info, cached here) and the
//! free-form tags on the rows it served for it. A screen keeps its own row and
//! picks and sends them with every ask, so nothing kept here belongs to a
//! screen: leaving a category leaves no trace of what was picked there.

use crate::models::stream::{CategoryInfo, TwitchStream};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Categories known at once; the least recently asked about goes first.
const MAX_POOLS: usize = 8;
const MAX_OFFERED: usize = 100;
const MAX_SEEN: usize = 400;
/// Twitch tags are at most 25 characters; anything far past that is not a tag.
const MAX_LABEL_CHARS: usize = 64;
const MAX_SELECTED: usize = 50;
const MAX_SHOWN: usize = 400;
/// Row labels that are neither the category's own tags nor picks.
const ROW_SEEN_CAP: usize = 30;
const MAX_LIMIT: u32 = 200;

/// The answer to one ask: the row (empty query) or the matches (a search).
/// `offer_typed` is the typed text when no match equals it, for "Filter by
/// tag". `retry_after_ms` is set while the category's own tags could not be
/// read: ask once more after that long.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct TagSuggestions {
    pub labels: Vec<String>,
    pub offer_typed: Option<String>,
    pub retry_after_ms: Option<u64>,
}

#[derive(Clone, Debug)]
struct Label {
    text: String,
    lower: String,
}

fn label(raw: &str) -> Option<Label> {
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > MAX_LABEL_CHARS {
        return None;
    }
    Some(Label { text: text.to_string(), lower: text.to_lowercase() })
}

/// Trimmed, valid, deduped case-insensitively, at most `cap`.
fn labels(raw: &[String], cap: usize) -> Vec<Label> {
    let mut seen = HashSet::new();
    raw.iter().filter_map(|r| label(r)).filter(|l| seen.insert(l.lower.clone())).take(cap).collect()
}

/// A category's key here: its name, trimmed and lowercased. The tag read is by
/// name and some desktop routes know only the name.
fn category_key(name: &str) -> String {
    name.trim().to_lowercase()
}

struct Pool {
    offered: Vec<Label>,
    seen: Vec<Label>,
    seen_index: HashSet<String>,
    /// The last answer and when it came; `None` inside means Twitch knows no
    /// such category, which is an answer too.
    info: Option<(Option<CategoryInfo>, Instant)>,
    /// The last read that failed (refused, unreachable, too slow).
    failed_at: Option<Instant>,
    /// One info read in flight per category.
    gate: Arc<tokio::sync::Mutex<()>>,
    touched: u64,
}

impl Pool {
    fn new(touched: u64) -> Self {
        Pool {
            offered: Vec::new(),
            seen: Vec::new(),
            seen_index: HashSet::new(),
            info: None,
            failed_at: None,
            gate: Arc::new(tokio::sync::Mutex::new(())),
            touched,
        }
    }

    fn is_fresh(&self, now: Instant, fresh: Duration) -> bool {
        self.info.as_ref().is_some_and(|(_, at)| now.saturating_duration_since(*at) < fresh)
    }
}

enum Cached {
    Fresh(Option<CategoryInfo>),
    BackingOff,
    Missing(Arc<tokio::sync::Mutex<()>>),
}

#[derive(Default)]
struct Pools {
    map: HashMap<String, Pool>,
    tick: u64,
}

impl Pools {
    /// The pool for `key`, made when missing.
    fn touch(&mut self, key: &str) -> &mut Pool {
        self.tick += 1;
        let tick = self.tick;
        if !self.map.contains_key(key) {
            self.map.insert(key.to_string(), Pool::new(tick));
            self.shrink(Some(key));
        }
        let pool = self.map.get_mut(key).expect("inserted above");
        pool.touched = tick;
        pool
    }

    /// Back to 8 pools: the least recently touched go first, but never one a
    /// read holds or waits on (its gate is shared then), nor `keep`. Runs when
    /// a pool is added and when a read ends.
    fn shrink(&mut self, keep: Option<&str>) {
        while self.map.len() > MAX_POOLS {
            let oldest = self
                .map
                .iter()
                .filter(|(k, pool)| Some(k.as_str()) != keep && Arc::strong_count(&pool.gate) == 1)
                .min_by_key(|(_, pool)| pool.touched)
                .map(|(k, _)| k.clone());
            match oldest {
                Some(oldest) => {
                    self.map.remove(&oldest);
                }
                None => break,
            }
        }
    }

    fn record(&mut self, rows: &[TwitchStream]) {
        for row in rows {
            let key = category_key(&row.game_name);
            let Some(tags) = row.tags.as_deref() else { continue };
            if key.is_empty() || tags.is_empty() {
                continue;
            }
            let pool = self.touch(&key);
            for tag in tags {
                if pool.seen.len() >= MAX_SEEN {
                    break;
                }
                if let Some(l) = label(tag) {
                    if pool.seen_index.insert(l.lower.clone()) {
                        pool.seen.push(l);
                    }
                }
            }
        }
    }

    fn set_info(&mut self, key: &str, info: Option<CategoryInfo>, now: Instant) {
        let own: Vec<String> =
            info.iter().flat_map(|i| i.tags.iter().flatten()).map(|t| t.localized_name.clone()).collect();
        let pool = self.touch(key);
        pool.offered = labels(&own, MAX_OFFERED);
        pool.info = Some((info, now));
        pool.failed_at = None;
    }

    fn set_failed(&mut self, key: &str, now: Instant) {
        self.touch(key).failed_at = Some(now);
    }

    fn cached(&mut self, key: &str, now: Instant, fresh: Duration, backoff: Option<Duration>) -> Cached {
        let pool = self.touch(key);
        if let Some((info, at)) = &pool.info {
            if now.saturating_duration_since(*at) < fresh {
                return Cached::Fresh(info.clone());
            }
        }
        if let (Some(backoff), Some(failed)) = (backoff, pool.failed_at) {
            if now.saturating_duration_since(failed) < backoff {
                return Cached::BackingOff;
            }
        }
        Cached::Missing(pool.gate.clone())
    }

    fn is_fresh(&self, key: &str, now: Instant, fresh: Duration) -> bool {
        self.map.get(key).is_some_and(|pool| pool.is_fresh(now, fresh))
    }

    /// Milliseconds until a failed info read may be tried again; `None` when
    /// the last read answered (found or not) or none has failed.
    fn retry_after(&self, key: &str, now: Instant, fresh: Duration, backoff: Duration) -> Option<u64> {
        let pool = self.map.get(key)?;
        if pool.is_fresh(now, fresh) {
            return None;
        }
        let failed = pool.failed_at?;
        let left = backoff.saturating_sub(now.saturating_duration_since(failed)).as_millis() as u64;
        Some(left.max(1000))
    }

    fn answer(&mut self, key: &str, query: &str, selected: &[String], shown: &[String], limit: u32) -> (Vec<String>, Option<String>) {
        let limit = limit.clamp(1, MAX_LIMIT) as usize;
        let selected = labels(selected, MAX_SELECTED);
        let shown = labels(shown, MAX_SHOWN);
        let q: String = query.trim().to_lowercase().chars().take(MAX_LABEL_CHARS).collect();
        let scratch = Pool::new(0);
        let pool: &Pool = if key.is_empty() { &scratch } else { self.touch(key) };
        if q.is_empty() {
            (row(pool, &selected, &shown, limit), None)
        } else {
            search(pool, &q, query, &selected, &shown, limit)
        }
    }
}

/// The screen's row: new picks first, then what it already shows, then the
/// category's own tags and up to 30 other tags, each label once. Nothing the
/// screen shows is ever dropped or moved.
fn row(pool: &Pool, selected: &[Label], shown: &[Label], limit: usize) -> Vec<String> {
    let shown_set: HashSet<&str> = shown.iter().map(|l| l.lower.as_str()).collect();
    let offered_set: HashSet<&str> = pool.offered.iter().map(|l| l.lower.as_str()).collect();
    let selected_set: HashSet<&str> = selected.iter().map(|l| l.lower.as_str()).collect();
    let mut row: Vec<&Label> = Vec::new();
    let mut in_row: HashSet<&str> = HashSet::new();
    for l in selected.iter().filter(|l| !shown_set.contains(l.lower.as_str())) {
        if in_row.insert(l.lower.as_str()) {
            row.push(l);
        }
    }
    for l in shown {
        if in_row.insert(l.lower.as_str()) {
            row.push(l);
        }
    }
    let keep = row.len();
    for l in &pool.offered {
        if in_row.insert(l.lower.as_str()) {
            row.push(l);
        }
    }
    let mut others = row
        .iter()
        .filter(|l| !offered_set.contains(l.lower.as_str()) && !selected_set.contains(l.lower.as_str()))
        .count();
    for l in &pool.seen {
        if others >= ROW_SEEN_CAP {
            break;
        }
        if in_row.insert(l.lower.as_str()) {
            row.push(l);
            others += 1;
        }
    }
    row.truncate(limit.max(keep));
    row.into_iter().map(|l| l.text.clone()).collect()
}

/// Labels containing the query, prefix matches first, then alphabetical.
fn search(pool: &Pool, q: &str, query: &str, selected: &[Label], shown: &[Label], limit: usize) -> (Vec<String>, Option<String>) {
    let mut once: HashSet<&str> = HashSet::new();
    let mut hits: Vec<&Label> = selected
        .iter()
        .chain(shown)
        .chain(&pool.offered)
        .chain(&pool.seen)
        .filter(|l| once.insert(l.lower.as_str()))
        .filter(|l| l.lower.contains(q))
        .collect();
    hits.sort_by(|a, b| {
        (!a.lower.starts_with(q))
            .cmp(&!b.lower.starts_with(q))
            .then_with(|| a.lower.cmp(&b.lower))
            .then_with(|| a.text.cmp(&b.text))
    });
    hits.truncate(limit);
    let exact = hits.iter().any(|l| l.lower == q);
    let typed: String = query.trim().chars().take(MAX_LABEL_CHARS).collect();
    (hits.into_iter().map(|l| l.text.clone()).collect(), if exact { None } else { Some(typed) })
}

type InfoFuture = Pin<Box<dyn Future<Output = anyhow::Result<Option<CategoryInfo>>> + Send>>;
type InfoFetch = Box<dyn Fn(String) -> InfoFuture + Send + Sync>;

struct TagStore {
    pools: Mutex<Pools>,
    fetch: InfoFetch,
    timeout: Duration,
    fresh: Duration,
    backoff: Duration,
}

impl TagStore {
    fn new(fetch: InfoFetch, timeout: Duration, fresh: Duration, backoff: Duration) -> Self {
        TagStore { pools: Mutex::new(Pools::default()), fetch, timeout, fresh, backoff }
    }

    fn lock(&self) -> MutexGuard<'_, Pools> {
        self.pools.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn record(&self, rows: &[TwitchStream]) {
        self.lock().record(rows);
    }

    /// The category's info, read at most once per `fresh` window and once at a
    /// time. With `respect_backoff`, a recent failure answers `None` at once.
    async fn info(&self, name: &str, respect_backoff: bool) -> Result<Option<CategoryInfo>, String> {
        let result = self.read_info(name, respect_backoff).await;
        // Every gate clone the read took is gone now, so a pool the map kept
        // over its size while this read held or waited on it can go.
        self.lock().shrink(None);
        result
    }

    async fn read_info(&self, name: &str, respect_backoff: bool) -> Result<Option<CategoryInfo>, String> {
        let key = category_key(name);
        if key.is_empty() {
            return Ok(None);
        }
        let backoff = respect_backoff.then_some(self.backoff);
        let cached = self.lock().cached(&key, Instant::now(), self.fresh, backoff);
        let gate = match cached {
            Cached::Fresh(info) => return Ok(info),
            Cached::BackingOff => return Ok(None),
            Cached::Missing(gate) => gate,
        };
        let held = gate.lock().await;
        // A read that finished while this one waited answers for it.
        let cached = self.lock().cached(&key, Instant::now(), self.fresh, backoff);
        match cached {
            Cached::Fresh(info) => return Ok(info),
            Cached::BackingOff => return Ok(None),
            Cached::Missing(_) => {}
        }
        let answer = tokio::time::timeout(self.timeout, (self.fetch)(name.trim().to_string())).await;
        let now = Instant::now();
        let result = {
            let mut pools = self.lock();
            match answer {
                Ok(Ok(found)) => {
                    pools.set_info(&key, found.clone(), now);
                    Ok(found)
                }
                Ok(Err(e)) => {
                    pools.set_failed(&key, now);
                    Err(e.to_string())
                }
                Err(_) => {
                    pools.set_failed(&key, now);
                    Err("Twitch did not answer in time".to_string())
                }
            }
        };
        drop(held);
        result
    }

    async fn suggestions(&self, name: &str, query: &str, selected: &[String], shown: &[String], limit: u32) -> TagSuggestions {
        let key = category_key(name);
        let fresh = key.is_empty() || self.lock().is_fresh(&key, Instant::now(), self.fresh);
        if !fresh {
            // Its failure is the retry hint below; the seen tags still answer.
            let _ = self.info(name, true).await;
        }
        let mut pools = self.lock();
        let (labels, offer_typed) = pools.answer(&key, query, selected, shown, limit);
        let retry_after_ms =
            if key.is_empty() { None } else { pools.retry_after(&key, Instant::now(), self.fresh, self.backoff) };
        TagSuggestions { labels, offer_typed, retry_after_ms }
    }
}

static STORE: LazyLock<TagStore> = LazyLock::new(|| {
    TagStore::new(
        Box::new(|name: String| -> InfoFuture {
            Box::pin(async move { crate::services::twitch_service::TwitchService::get_category_info(&name).await })
        }),
        Duration::from_secs(8),
        Duration::from_secs(600),
        Duration::from_secs(30),
    )
});

/// Notes the tags on rows served for a category (after hidden channels left).
pub fn record_streams(rows: &[TwitchStream]) {
    STORE.record(rows);
}

/// `get_category_info` through the cache: one read per category at a time and
/// per ten minutes. Always tries, whatever a recent failure was.
pub async fn category_info(name: &str) -> Result<Option<CategoryInfo>, String> {
    STORE.info(name, false).await
}

/// The row (empty `query`) or the matches for a typed search.
pub async fn suggestions(name: &str, query: &str, selected: &[String], shown: &[String], limit: u32) -> TagSuggestions {
    STORE.suggestions(name, query, selected, shown, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stream::CategoryTag;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn info(name: &str, tags: &[&str]) -> CategoryInfo {
        CategoryInfo {
            id: "1".into(),
            name: name.into(),
            display_name: None,
            description: None,
            followers_count: None,
            box_art_url: None,
            tags: Some(tags.iter().map(|t| CategoryTag { id: t.to_string(), localized_name: t.to_string() }).collect()),
        }
    }

    fn stream(game: &str, tags: &[&str]) -> TwitchStream {
        TwitchStream {
            id: "s".into(),
            user_id: "u".into(),
            user_name: "U".into(),
            user_login: "u".into(),
            title: "t".into(),
            viewer_count: 1,
            game_id: "1".into(),
            game_name: game.into(),
            thumbnail_url: String::new(),
            started_at: String::new(),
            broadcaster_type: None,
            profile_image_url: None,
            is_live: Some(true),
            tags: Some(strings(tags)),
            language: None,
            drops_earnable: false,
        }
    }

    fn pools(offered: &[&str], seen: &[&str]) -> Pools {
        let mut p = Pools::default();
        p.set_info("art", Some(info("Art", offered)), Instant::now());
        p.record(&[stream("Art", seen)]);
        p
    }

    fn row_of(p: &mut Pools, selected: &[&str], shown: &[&str], limit: u32) -> Vec<String> {
        p.answer("art", "", &strings(selected), &strings(shown), limit).0
    }

    #[test]
    fn row_puts_new_picks_first_then_shown_then_offered_then_seen() {
        let mut p = pools(&["English", "Chill"], &["Cozy", "english"]);
        assert_eq!(row_of(&mut p, &[], &[], 200), strings(&["English", "Chill", "Cozy"]));
        // A pick from the sheet that the row lacks goes first; what is shown keeps its place.
        assert_eq!(
            row_of(&mut p, &["ARAM"], &["English", "Chill", "Cozy"], 200),
            strings(&["ARAM", "English", "Chill", "Cozy"])
        );
        // Unpicked, it stays where it is.
        assert_eq!(
            row_of(&mut p, &[], &["ARAM", "English", "Chill", "Cozy"], 200),
            strings(&["ARAM", "English", "Chill", "Cozy"])
        );
    }

    #[test]
    fn row_never_drops_or_moves_what_is_shown() {
        let mut p = pools(&["A"], &[]);
        let shown: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        let shown_refs: Vec<&str> = shown.iter().map(|s| s.as_str()).collect();
        let answer = row_of(&mut p, &[], &shown_refs, 5);
        assert_eq!(&answer[..40], &shown[..]);
    }

    #[test]
    fn row_caps_only_the_seen_tags() {
        let seen: Vec<String> = (0..50).map(|i| format!("s{i}")).collect();
        let seen_refs: Vec<&str> = seen.iter().map(|s| s.as_str()).collect();
        let mut p = pools(&["O1", "O2"], &seen_refs);
        let answer = row_of(&mut p, &[], &[], 200);
        assert_eq!(answer.len(), 2 + 30);
        assert_eq!(&answer[..2], &strings(&["O1", "O2"])[..]);
    }

    #[test]
    fn late_offered_still_joins_the_row() {
        let seen: Vec<String> = (0..30).map(|i| format!("s{i}")).collect();
        let seen_refs: Vec<&str> = seen.iter().map(|s| s.as_str()).collect();
        let mut p = Pools::default();
        p.record(&[stream("Art", &seen_refs)]);
        let first = row_of(&mut p, &[], &[], 200);
        assert_eq!(first, seen);
        p.set_info("art", Some(info("Art", &["English"])), Instant::now());
        let first_refs: Vec<&str> = first.iter().map(|s| s.as_str()).collect();
        let next = row_of(&mut p, &[], &first_refs, 200);
        assert_eq!(&next[..30], &seen[..]);
        assert_eq!(next[30], "English");
    }

    #[test]
    fn search_ranks_prefix_first_then_alphabetical() {
        let mut p = pools(&["Speedrun", "Any%"], &["NoSpeed", "speedrun", "Speedy"]);
        let (labels, _) = p.answer("art", "spee", &[], &[], 14);
        assert_eq!(labels, strings(&["Speedrun", "Speedy", "NoSpeed"]));
        let (labels, _) = p.answer("art", "spee", &[], &[], 1);
        assert_eq!(labels, strings(&["Speedrun"]));
    }

    #[test]
    fn exact_match_hides_typed_offer() {
        let mut p = pools(&["English"], &[]);
        assert_eq!(p.answer("art", " english ", &[], &[], 14).1, None);
        assert_eq!(p.answer("art", "engl", &[], &[], 14).1, Some("engl".to_string()));
        assert_eq!(p.answer("art", "ARAM", &[], &[], 14), (Vec::new(), Some("ARAM".to_string())));
    }

    #[test]
    fn pools_stay_bounded() {
        let mut p = Pools::default();
        for i in 0..9 {
            p.record(&[stream(&format!("Game{i}"), &["x"])]);
        }
        assert_eq!(p.map.len(), MAX_POOLS);
        assert!(!p.map.contains_key("game0"));
        let many: Vec<String> = (0..450).map(|i| format!("t{i}")).collect();
        let many_refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        p.record(&[stream("Game8", &many_refs)]);
        assert_eq!(p.map["game8"].seen.len(), MAX_SEEN);
        let long = "x".repeat(65);
        p.record(&[stream("Game7", &[long.as_str()])]);
        assert!(p.map["game7"].seen.iter().all(|l| l.text != long));
        // Every pool held by a read: the map grows past 8, then shrinks back
        // once the reads end.
        let mut held: Vec<_> = p.map.values().map(|pool| pool.gate.clone()).collect();
        p.record(&[stream("Game9", &["x"])]);
        held.push(p.map["game9"].gate.clone());
        p.record(&[stream("Game10", &["x"])]);
        assert_eq!(p.map.len(), MAX_POOLS + 2);
        drop(held);
        p.shrink(None);
        assert_eq!(p.map.len(), MAX_POOLS);
    }

    #[test]
    fn waiting_pool_is_not_evicted() {
        let mut p = Pools::default();
        p.record(&[stream("Game0", &["x"])]);
        let gate = p.map["game0"].gate.clone();
        for i in 1..9 {
            p.record(&[stream(&format!("Game{i}"), &["x"])]);
        }
        assert!(p.map.contains_key("game0"));
        assert!(!p.map.contains_key("game1"));
        drop(gate);
    }

    #[test]
    fn record_groups_rows_by_category() {
        let mut p = Pools::default();
        p.record(&[stream("Art", &["Cozy"]), stream("Chess", &["Blitz"]), stream("", &["Lost"])]);
        assert_eq!(p.map["art"].seen.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["Cozy"]);
        assert_eq!(p.map["chess"].seen.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["Blitz"]);
        assert!(!p.map.contains_key(""));
    }

    /// What the fake info read answers.
    #[derive(Clone, Copy)]
    enum Outcome {
        Found(&'static [&'static str]),
        Unknown,
        Fails,
    }

    fn counting_store(calls: Arc<AtomicUsize>, wait: Duration, outcome: Outcome, timeout: Duration, backoff: Duration) -> TagStore {
        TagStore::new(
            Box::new(move |name: String| -> InfoFuture {
                calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    tokio::time::sleep(wait).await;
                    match outcome {
                        Outcome::Found(tags) => Ok(Some(info(&name, tags))),
                        Outcome::Unknown => Ok(None),
                        Outcome::Fails => Err(anyhow::anyhow!("Twitch answered HTTP 503")),
                    }
                })
            }),
            timeout,
            Duration::from_secs(600),
            backoff,
        )
    }

    #[tokio::test]
    async fn info_reads_once_for_concurrent_callers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = counting_store(calls.clone(), Duration::from_millis(50), Outcome::Found(&["English"]), Duration::from_secs(5), Duration::from_secs(30));
        let (a, b) = tokio::join!(store.info("Art", true), store.info("Art", false));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(a.unwrap().is_some() && b.unwrap().is_some());
        let answer = store.suggestions("Art", "", &[], &[], 200).await;
        assert_eq!(answer.labels, strings(&["English"]));
        assert_eq!(answer.retry_after_ms, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn info_failure_backs_off_suggestions_but_not_the_header() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = counting_store(calls.clone(), Duration::ZERO, Outcome::Fails, Duration::from_secs(5), Duration::from_secs(3600));
        let first = store.suggestions("Art", "", &[], &[], 200).await;
        assert!(first.retry_after_ms.is_some());
        let _ = store.suggestions("Art", "", &[], &[], 200).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = store.info("Art", false).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unknown_category_is_fresh_and_not_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = counting_store(calls.clone(), Duration::ZERO, Outcome::Unknown, Duration::from_secs(5), Duration::from_secs(30));
        let first = store.suggestions("Nope", "", &[], &[], 200).await;
        assert_eq!(first.retry_after_ms, None);
        let _ = store.suggestions("Nope", "", &[], &[], 200).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn info_timeout_fails_and_asks_for_a_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = counting_store(calls.clone(), Duration::from_millis(500), Outcome::Found(&["English"]), Duration::from_millis(50), Duration::from_secs(30));
        assert!(store.info("Art", true).await.is_err());
        let answer = store.suggestions("Art", "", &[], &[], 200).await;
        assert!(answer.labels.is_empty());
        assert!(answer.retry_after_ms.is_some_and(|ms| ms >= 1000));
    }

    #[tokio::test]
    async fn empty_name_never_fetches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let store = counting_store(calls.clone(), Duration::ZERO, Outcome::Found(&["English"]), Duration::from_secs(5), Duration::from_secs(30));
        let answer = store.suggestions("  ", "aram", &strings(&["Cozy"]), &[], 14).await;
        assert_eq!(answer, TagSuggestions { labels: Vec::new(), offer_typed: Some("aram".into()), retry_after_ms: None });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    /// The source of one function: from its head to the first line that closes
    /// it at `indent`. CRLF is normalized (the Windows checkout is CRLF).
    fn body(src: &str, head: &str, indent: &str) -> String {
        let src = src.replace("\r\n", "\n");
        let start = src.find(head).unwrap_or_else(|| panic!("{head} not found"));
        let close = format!("\n{indent}}}\n");
        let end = src[start..].find(&close).unwrap_or_else(|| panic!("{head} never closes")) + start;
        src[start..end].to_string()
    }

    #[test]
    fn call_sites_record_and_stamp_the_category_rows() {
        let commands = include_str!("../commands/twitch.rs");
        for head in [
            "pub async fn get_streams_by_game(",
            "pub async fn get_streams_by_game_name(",
            "pub async fn get_streams_by_game_id(",
            "pub async fn get_streams_by_game_with_tags(",
        ] {
            assert!(body(commands, head, "").contains("category_tags::record_streams(&streams)"), "{head}");
        }
        assert!(body(commands, "pub async fn get_streams_by_game_with_tags(", "").contains("stamp_drops(&mut streams)"));
        assert!(body(commands, "pub async fn get_category_info(", "").contains("category_tags::category_info(&game_name)"));
    }

    #[test]
    fn call_sites_read_tags_through_the_public_door() {
        let service = include_str!("twitch_service.rs");
        let read = body(service, "pub async fn get_streams_by_game_with_tags(", "    ");
        assert!(read.contains("Self::gql_public_read("));
        assert!(!read.contains("TWITCH_GQL_CLIENT_ID"));
    }
}
