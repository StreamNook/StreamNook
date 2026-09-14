use crate::models::chat_layout::ChatMessage;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::collections::HashSet;
use std::sync::OnceLock;
use tauri::Emitter;
use tokio::sync::Mutex;

const MAX_USERS: usize = 1000;
const MAX_MESSAGES_PER_USER: usize = 200;

#[derive(Clone)]
struct LruEntry {
    messages: VecDeque<UserMessageSummary>,
    last_access: u64,
}

/// User message history service with LRU cache.
///
/// Stores a compact summary of each chat message (id, content, timestamp, color)
/// rather than the full `ChatMessage` struct. Profile cards only need those four
/// fields to render their message timeline; badges, segments, and tags would be
/// 1-3 KB per entry of wasted RAM.
///
/// The caps are a ceiling, not an allocation: a user costs only the messages
/// they actually sent. A summary is ~200 bytes, so a busy channel with a couple
/// dozen heavy chatters and several hundred light ones sits around 1-2 MB. The
/// absolute worst case (every one of 1000 users hitting 200 messages) is ~40 MB
/// and is not reachable in practice.
///
/// These caps are deliberately generous because they are what the profile card's
/// message timeline can show for a channel where the viewer is NOT a moderator.
/// Twitch only serves its own multi-year archive (`viewerCardModLogs`) to
/// moderators of that channel, so for everywhere else this in-session buffer is
/// the history. The previous 300x20 caps truncated heavy chatters to their last
/// 20 messages and evicted early speakers entirely.
pub struct UserMessageHistoryService {
    cache: Mutex<HashMap<String, LruEntry>>,
    // Atomic rather than a second mutex: add_message runs per chat message and
    // paid two lock acquisitions where a fetch_add does.
    access_counter: std::sync::atomic::AtomicU64,
}

static INSTANCE: OnceLock<UserMessageHistoryService> = OnceLock::new();

/// Users with an open profile card somewhere: every new message from one of
/// them is pushed as `user-history-message` instead of the card polling the
/// service every 2.5 s. Keyed the same way the cache is (`historyKey`).
static WATCHED_USERS: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Event name for pushed messages. Payload: `{ user_key, message }`.
pub const HISTORY_EVENT: &str = "user-history-message";

#[derive(Serialize, Clone)]
struct HistoryPush<'a> {
    user_key: &'a str,
    message: &'a UserMessageSummary,
}

fn watched() -> &'static std::sync::Mutex<HashSet<String>> {
    WATCHED_USERS.get_or_init(|| std::sync::Mutex::new(HashSet::new()))
}

impl UserMessageHistoryService {
    pub fn global() -> &'static UserMessageHistoryService {
        INSTANCE.get_or_init(|| UserMessageHistoryService {
            cache: Mutex::new(HashMap::with_capacity(MAX_USERS)),
            access_counter: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// (users, messages) held, or `None` if the lock is busy. Diagnostics for
    /// the resource line.
    pub fn cache_counts(&self) -> Option<(usize, usize)> {
        self.cache
            .try_lock()
            .ok()
            .map(|c| (c.len(), c.values().map(|e| e.messages.len()).sum()))
    }

    pub fn set_app_handle(app: tauri::AppHandle) {
        let _ = APP.set(app);
    }

    pub fn watch_user(user_key: &str) {
        if let Ok(mut w) = watched().lock() {
            w.insert(user_key.to_string());
        }
    }

    pub fn unwatch_user(user_key: &str) {
        if let Ok(mut w) = watched().lock() {
            w.remove(user_key);
        }
    }

    pub async fn add_message(&self, user_id: &str, message: &ChatMessage) {
        let summary: UserMessageSummary = message.into();
        // Push to open cards before the lock: an emit is cheap and the watched
        // set is tiny, so this costs the chat path one HashSet lookup.
        let pushed = watched().lock().map(|w| w.contains(user_id)).unwrap_or(false);
        if pushed {
            if let Some(app) = APP.get() {
                let _ = app.emit(HISTORY_EVENT, HistoryPush { user_key: user_id, message: &summary });
            }
        }
        let mut cache = self.cache.lock().await;
        let current_access = self
            .access_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;

        let entry = cache
            .entry(user_id.to_string())
            .or_insert_with(|| LruEntry {
                // Grown on demand rather than preallocated: most chatters send a
                // handful of messages, so reserving the full cap per user would
                // cost far more than the entries themselves.
                messages: VecDeque::new(),
                last_access: current_access,
            });

        entry.last_access = current_access;

        // pop_front is O(1); a Vec's remove(0) shifted every remaining element on
        // each message once a user hit the cap, which matters at 200 per user in
        // a fast chat.
        if entry.messages.len() >= MAX_MESSAGES_PER_USER {
            entry.messages.pop_front();
        }
        entry.messages.push_back(summary);

        if cache.len() > MAX_USERS {
            self.evict_lru(&mut cache);
        }
    }

    pub async fn get_history(&self, user_id: &str) -> Vec<UserMessageSummary> {
        let mut cache = self.cache.lock().await;
        let current_access = self
            .access_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;

        if let Some(entry) = cache.get_mut(user_id) {
            entry.last_access = current_access;
            entry.messages.iter().cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub async fn get_history_limited(
        &self,
        user_id: &str,
        limit: usize,
    ) -> Vec<UserMessageSummary> {
        let history = self.get_history(user_id).await;
        if history.len() > limit {
            history.into_iter().rev().take(limit).rev().collect()
        } else {
            history
        }
    }

    pub async fn clear_user(&self, user_id: &str) {
        let mut cache = self.cache.lock().await;
        cache.remove(user_id);
    }

    pub async fn clear_all(&self) {
        let mut cache = self.cache.lock().await;
        cache.clear();
    }

    pub async fn user_count(&self) -> usize {
        self.cache.lock().await.len()
    }

    fn evict_lru(&self, cache: &mut HashMap<String, LruEntry>) {
        let evict_count = cache.len() / 10;
        if evict_count == 0 {
            return;
        }

        let mut entries: Vec<(String, u64)> = cache
            .iter()
            .map(|(k, v)| (k.clone(), v.last_access))
            .collect();
        entries.sort_by_key(|(_, access)| *access);

        for (user_id, _) in entries.into_iter().take(evict_count) {
            cache.remove(&user_id);
        }
    }
}

/// Compact per-message record exposed to the frontend's profile-card timeline.
/// `timestamp` is unix-ms as a String (matches `tmi-sent-ts`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserMessageSummary {
    pub id: String,
    pub content: String,
    pub timestamp: String,
    pub color: Option<String>,
}

impl From<&ChatMessage> for UserMessageSummary {
    fn from(msg: &ChatMessage) -> Self {
        UserMessageSummary {
            id: msg.id.clone(),
            content: msg.content.clone(),
            timestamp: msg.timestamp.clone(),
            color: msg.color.clone(),
        }
    }
}
