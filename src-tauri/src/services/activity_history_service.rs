// MultiChat Activity feed history: follows, subs, raids, gifts, super chats and
// the rest, persisted so a streamer's past activity is still there on reopen.
//
// One store for every window. Each MultiChat window used to keep its own copy
// in localStorage and re-read, merge and rewrite the whole list (up to 2,500
// events) on every single event, on the UI thread. Here an append is a hash
// lookup and a push, and the disk write is debounced.
//
// Events are stored as opaque JSON so this layer never has to track the
// frontend's ActivityEvent shape; only `id` (de-dup) and `channel` (the
// composite "<provider>:<channel>" source key, for caps and purges) are read.
//
// Retention: newest PER_CHANNEL_CAP per source, so one busy channel cannot
// evict another's history, and TOTAL_CAP overall.

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

const HISTORY_FILE: &str = "activity_history.json";
const PER_CHANNEL_CAP: usize = 200;
const TOTAL_CAP: usize = 2500;
const FLUSH_DEBOUNCE: Duration = Duration::from_secs(2);

/// Newest first. None until seeded from disk (the session's one full read).
static STORE: Mutex<Option<History>> = Mutex::new(None);
static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);
/// A flush task is scheduled. It ends once nothing is left to write, so an idle
/// feed costs no wakeups at all.
static FLUSH_PENDING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct History {
    events: Vec<Value>,
    ids: HashSet<String>,
    /// Events held per source, so an append only walks the list when a cap is
    /// actually exceeded.
    per_source: HashMap<String, usize>,
}

fn id_of(event: &Value) -> Option<&str> {
    event.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())
}

fn source_of(event: &Value) -> String {
    event
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
}

impl History {
    fn from_events(events: Vec<Value>) -> Self {
        let mut history = History::default();
        history.extend_older(events);
        history
    }

    /// Append `older` behind what is already held (skipping known ids), then
    /// re-apply the caps.
    fn extend_older(&mut self, older: Vec<Value>) {
        for event in older {
            if let Some(id) = id_of(&event) {
                if !self.ids.insert(id.to_string()) {
                    continue;
                }
            }
            *self.per_source.entry(source_of(&event)).or_insert(0) += 1;
            self.events.push(event);
        }
        self.cap();
    }

    /// Put one event at the front. None when its id is already held,
    /// otherwise the ids the caps evicted to make room.
    fn prepend(&mut self, event: Value) -> Option<Vec<String>> {
        if let Some(id) = id_of(&event) {
            if !self.ids.insert(id.to_string()) {
                return None;
            }
        }
        let held = self.per_source.entry(source_of(&event)).or_insert(0);
        *held += 1;
        let over = *held > PER_CHANNEL_CAP;
        self.events.insert(0, event);
        if over || self.events.len() > TOTAL_CAP {
            Some(self.cap())
        } else {
            Some(Vec::new())
        }
    }

    /// Apply the retention caps, returning the ids of the events dropped.
    fn cap(&mut self) -> Vec<String> {
        let mut evicted = Vec::new();
        if self.events.len() <= PER_CHANNEL_CAP {
            return evicted;
        }
        let mut per_source: HashMap<String, usize> = HashMap::new();
        let mut kept = Vec::with_capacity(self.events.len().min(TOTAL_CAP));
        for event in self.events.drain(..) {
            let count = per_source.entry(source_of(&event)).or_insert(0);
            if kept.len() >= TOTAL_CAP || *count >= PER_CHANNEL_CAP {
                if let Some(id) = id_of(&event) {
                    evicted.push(id.to_string());
                }
                continue;
            }
            *count += 1;
            kept.push(event);
        }
        self.events = kept;
        self.per_source = per_source;
        for id in &evicted {
            self.ids.remove(id);
        }
        evicted
    }

    fn retain_sources(&mut self, drop: &HashSet<String>) -> bool {
        let before = self.events.len();
        self.events.retain(|e| !drop.contains(&source_of(e)));
        let changed = self.events.len() != before;
        if changed {
            self.reindex();
        }
        changed
    }

    fn reindex(&mut self) {
        self.ids = self
            .events
            .iter()
            .filter_map(|e| id_of(e).map(String::from))
            .collect();
        self.per_source.clear();
        for event in &self.events {
            *self.per_source.entry(source_of(event)).or_insert(0) += 1;
        }
    }
}

fn storage_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {e}"))?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create app data directory: {e}"))?;
    }
    Ok(dir.join(HISTORY_FILE))
}

fn with_history<R>(app: &AppHandle, f: impl FnOnce(&mut History) -> R) -> Result<R, String> {
    let mut guard = STORE.lock().map_err(|e| e.to_string())?;
    if guard.is_none() {
        let path = storage_path(app)?;
        let events: Vec<Value> = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let _ = STORE_PATH.set(path);
        *guard = Some(History::from_events(events));
    }
    Ok(f(guard.as_mut().expect("seeded above")))
}

fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
    schedule_flush();
}

fn schedule_flush() {
    if FLUSH_PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tauri::async_runtime::spawn(async {
        loop {
            tokio::time::sleep(FLUSH_DEBOUNCE).await;
            // flush_now restores DIRTY itself when a write fails.
            let _ = tokio::task::spawn_blocking(flush_now).await;
            FLUSH_PENDING.store(false, Ordering::Release);
            // An append that raced the flush left DIRTY set without scheduling;
            // pick it up here rather than waiting for the next event.
            if !DIRTY.load(Ordering::Acquire)
                || FLUSH_PENDING
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
            {
                return;
            }
        }
    });
}

/// Synchronous write-if-dirty, also used by the exit path.
pub fn flush_now() -> Result<(), String> {
    if !DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let Some(path) = STORE_PATH.get() else {
        return Ok(());
    };
    let json = {
        let guard = STORE.lock().map_err(|e| {
            DIRTY.store(true, Ordering::Release);
            e.to_string()
        })?;
        let Some(history) = guard.as_ref() else {
            return Ok(());
        };
        serde_json::to_string(&history.events).map_err(|e| {
            DIRTY.store(true, Ordering::Release);
            format!("Failed to serialize activity history: {e}")
        })?
    };
    fs::write(path, json).map_err(|e| {
        DIRTY.store(true, Ordering::Release);
        format!("Failed to write activity history: {e}")
    })
}

/// Every stored event, newest first.
pub fn load(app: &AppHandle) -> Result<Vec<Value>, String> {
    with_history(app, |h| h.events.clone())
}

#[derive(serde::Serialize)]
pub struct Appended {
    /// False when the id was already stored (sources echo the same event).
    pub added: bool,
    /// Events the retention caps dropped to make room, so a window showing
    /// them can drop them too.
    pub evicted: Vec<String>,
}

/// Record one event.
pub fn append(app: &AppHandle, event: Value) -> Result<Appended, String> {
    let evicted = with_history(app, |h| h.prepend(event))?;
    if evicted.is_some() {
        mark_dirty();
    }
    Ok(Appended { added: evicted.is_some(), evicted: evicted.unwrap_or_default() })
}

/// Merge older events in behind the stored ones: the one-time move of a
/// window's localStorage history into this store.
pub fn import(app: &AppHandle, events: Vec<Value>) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    with_history(app, |h| h.extend_older(events))?;
    mark_dirty();
    Ok(())
}

/// Forget the given composite source keys.
pub fn purge(app: &AppHandle, source_keys: Vec<String>) -> Result<(), String> {
    let drop: HashSet<String> = source_keys.into_iter().map(|k| k.to_lowercase()).collect();
    if with_history(app, |h| h.retain_sources(&drop))? {
        mark_dirty();
    }
    Ok(())
}

pub fn clear(app: &AppHandle) -> Result<(), String> {
    with_history(app, |h| *h = History::default())?;
    mark_dirty();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(id: &str, channel: &str) -> Value {
        json!({ "id": id, "channel": channel, "kind": "sub" })
    }

    fn ids(h: &History) -> Vec<String> {
        h.events.iter().map(|e| id_of(e).unwrap().to_string()).collect()
    }

    #[test]
    fn prepends_newest_first_and_drops_echoes() {
        let mut h = History::default();
        assert!(h.prepend(ev("a", "twitch:x")).is_some());
        assert!(h.prepend(ev("b", "twitch:x")).is_some());
        assert!(h.prepend(ev("a", "twitch:x")).is_none());
        assert_eq!(ids(&h), vec!["b", "a"]);
    }

    #[test]
    fn one_busy_source_cannot_evict_another() {
        let mut h = History::default();
        h.prepend(ev("quiet", "kick:y"));
        let mut evicted = Vec::new();
        for i in 0..(PER_CHANNEL_CAP + 50) {
            evicted.extend(h.prepend(ev(&format!("busy{i}"), "twitch:x")).unwrap());
        }
        // Exactly the 50 oldest of the busy source were reported, oldest first.
        assert_eq!(evicted.len(), 50);
        assert_eq!(evicted[0], "busy0");
        let busy = h.events.iter().filter(|e| source_of(e) == "twitch:x").count();
        assert_eq!(busy, PER_CHANNEL_CAP);
        assert!(h.events.iter().any(|e| id_of(e) == Some("quiet")));
        // The newest of the busy source survive, the oldest go.
        assert_eq!(id_of(&h.events[0]), Some(format!("busy{}", PER_CHANNEL_CAP + 49).as_str()));
        assert!(!h.ids.contains("busy0"));
    }

    #[test]
    fn source_keys_compare_case_insensitively() {
        let mut h = History::default();
        for i in 0..(PER_CHANNEL_CAP + 1) {
            let channel = if i % 2 == 0 { "twitch:X" } else { "twitch:x" };
            h.prepend(ev(&format!("e{i}"), channel));
        }
        assert_eq!(h.events.len(), PER_CHANNEL_CAP);
    }

    #[test]
    fn the_total_ceiling_holds_across_sources() {
        let mut h = History::default();
        for s in 0..20 {
            for i in 0..PER_CHANNEL_CAP {
                h.prepend(ev(&format!("s{s}e{i}"), &format!("twitch:c{s}")));
            }
        }
        assert_eq!(h.events.len(), TOTAL_CAP);
        assert_eq!(h.ids.len(), TOTAL_CAP);
        assert_eq!(h.per_source.values().sum::<usize>(), TOTAL_CAP);
    }

    #[test]
    fn import_goes_behind_and_skips_known_ids() {
        let mut h = History::default();
        h.prepend(ev("live", "twitch:x"));
        h.extend_older(vec![ev("old1", "twitch:x"), ev("live", "twitch:x"), ev("old2", "kick:y")]);
        assert_eq!(ids(&h), vec!["live", "old1", "old2"]);
    }

    #[test]
    fn purge_removes_only_the_named_sources() {
        let mut h = History::from_events(vec![ev("a", "twitch:x"), ev("b", "Kick:Y"), ev("c", "twitch:z")]);
        assert!(h.retain_sources(&HashSet::from(["kick:y".to_string()])));
        assert_eq!(ids(&h), vec!["a", "c"]);
        assert!(!h.ids.contains("b"));
        assert!(!h.retain_sources(&HashSet::from(["nope:n".to_string()])));
    }

    #[test]
    fn events_without_an_id_are_kept_but_never_deduped() {
        let mut h = History::default();
        assert!(h.prepend(json!({ "channel": "twitch:x" })).is_some());
        assert!(h.prepend(json!({ "channel": "twitch:x" })).is_some());
        assert_eq!(h.events.len(), 2);
    }
}
