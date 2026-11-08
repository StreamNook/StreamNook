//! Whether a whisper partner is live right now, for the inbox rows and the
//! conversation header. It reads what StreamNook already knows: the Following
//! list's live rows, and a small cache filled when a conversation opens.
//! Nothing here runs on a timer. A list read only copies the Following list's
//! answer; opening a conversation asks Twitch at most once per person every
//! two minutes.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::Serialize;

use crate::models::stream::TwitchStream;
use crate::services::whisper_inbox::RowMeta;
use crate::services::whisper_store::Scope;

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct LiveNow {
    pub title: String,
    pub category: String,
    pub viewers: u32,
}

/// A looked-up answer is reused this long before a conversation opening asks again.
pub const FRESH_MS: i64 = 120_000;
/// A looked-up "live" older than this is not shown: the stream may have ended.
const STALE_MS: i64 = 600_000;
/// People remembered at most; the oldest answers drop first.
const CAP: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// Copied from the Following list, which lists every live followed channel.
    Followed,
    /// Asked of Twitch for this one person.
    Lookup,
}

#[derive(Clone, Debug)]
struct Known {
    live: Option<LiveNow>,
    at_ms: i64,
    source: Source,
}

#[derive(Default)]
struct Cache {
    known: HashMap<String, Known>,
    asking: HashSet<String>,
}

impl Cache {
    /// Record an answer; returns whether what is shown for them changed.
    fn remember(&mut self, login: &str, live: Option<LiveNow>, source: Source, now_ms: i64) -> bool {
        let key = login.to_lowercase();
        let before = self.shown(&key, now_ms);
        self.known.insert(key.clone(), Known { live, at_ms: now_ms, source });
        if self.known.len() > CAP {
            if let Some(oldest) = self.known.iter().min_by_key(|(_, k)| k.at_ms).map(|(k, _)| k.clone()) {
                self.known.remove(&oldest);
            }
        }
        before != self.shown(&key, now_ms)
    }

    /// What a row shows: a looked-up "live" only while it is recent.
    fn shown(&self, key: &str, now_ms: i64) -> Option<LiveNow> {
        let k = self.known.get(key)?;
        if k.source == Source::Lookup && now_ms - k.at_ms > STALE_MS {
            return None;
        }
        k.live.clone()
    }

    /// Someone the Following list said was live is missing from it now: they ended.
    fn ended_on_following(&mut self, key: &str, now_ms: i64) -> bool {
        match self.known.get(key) {
            Some(k) if k.source == Source::Followed && k.live.is_some() => self.remember(key, None, Source::Followed, now_ms),
            _ => false,
        }
    }

    fn fresh(&self, key: &str, now_ms: i64) -> bool {
        self.known.get(key).is_some_and(|k| now_ms - k.at_ms < FRESH_MS)
    }
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache::default()));

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn live_of(s: &TwitchStream) -> LiveNow {
    LiveNow { title: s.title.clone(), category: s.game_name.clone(), viewers: s.viewer_count }
}

/// What is known for this login, without asking anyone.
pub fn cached(login: &str) -> Option<LiveNow> {
    CACHE.lock().unwrap_or_else(|p| p.into_inner()).shown(&login.to_lowercase(), now_ms())
}

/// Rows about to be drawn: copy the Following list's live answer for each of
/// these people into the cache (no network), then stamp each row.
pub async fn fill(rows: &mut [RowMeta]) {
    let now = now_ms();
    for row in rows.iter() {
        let found = crate::services::home_snapshot::followed_live_row(&row.user_login).await;
        let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        match found {
            Some(stream) => {
                cache.remember(&row.user_login, Some(live_of(&stream)), Source::Followed, now);
            }
            None => {
                cache.ended_on_following(&row.user_login.to_lowercase(), now);
            }
        }
    }
    let cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    for row in rows.iter_mut() {
        row.live = cache.shown(&row.user_login.to_lowercase(), now);
    }
}

/// Ask for one person now: the Following list first, else Twitch once (not
/// again within `FRESH_MS`, one question in flight per person). Returns
/// whether what is shown for them changed.
pub async fn refresh_one(login: &str, user_id: &str) -> bool {
    let key = login.to_lowercase();
    if let Some(stream) = crate::services::home_snapshot::followed_live_row(login).await {
        return CACHE.lock().unwrap_or_else(|p| p.into_inner()).remember(&key, Some(live_of(&stream)), Source::Followed, now_ms());
    }
    {
        let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
        if user_id.is_empty() || cache.fresh(&key, now_ms()) || !cache.asking.insert(key.clone()) {
            return false;
        }
    }
    let answer = crate::services::twitch_service::TwitchService::get_streams_by_user_ids(&[user_id.to_string()]).await;
    let mut cache = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    cache.asking.remove(&key);
    match answer {
        Ok(rows) => {
            let live = rows.iter().find(|s| s.user_id == user_id).map(live_of);
            cache.remember(&key, live, Source::Lookup, now_ms())
        }
        Err(e) => {
            log::debug!("[WhisperLive] live lookup failed: {e}");
            false
        }
    }
}

/// `refresh_one` in the background; a changed answer reaches the windows as
/// that conversation's update (its row carries `live`).
pub fn refresh_soon(scope: Scope, key: String, login: String, user_id: String) {
    tauri::async_runtime::spawn(async move {
        if refresh_one(&login, &user_id).await {
            crate::services::whisper_inbox::emit_conversation(&scope, &key);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(viewers: u32) -> Option<LiveNow> {
        Some(LiveNow { title: "a title".into(), category: "Just Chatting".into(), viewers })
    }

    #[test]
    fn a_looked_up_live_answer_stops_showing_when_it_is_old() {
        let mut c = Cache::default();
        assert!(c.remember("Alpha", live(10), Source::Lookup, 1_000));
        assert_eq!(c.shown("alpha", 1_000 + STALE_MS), live(10));
        assert_eq!(c.shown("alpha", 1_001 + STALE_MS), None);
    }

    #[test]
    fn the_following_list_dropping_someone_means_they_ended() {
        let mut c = Cache::default();
        c.remember("alpha", live(5), Source::Followed, 1_000);
        assert!(c.ended_on_following("alpha", 2_000));
        assert_eq!(c.shown("alpha", 2_000), None);
        // Someone only looked up is not judged by the Following list.
        c.remember("beta", live(5), Source::Lookup, 1_000);
        assert!(!c.ended_on_following("beta", 2_000));
        assert_eq!(c.shown("beta", 2_000), live(5));
    }

    #[test]
    fn an_answer_is_fresh_for_two_minutes() {
        let mut c = Cache::default();
        c.remember("alpha", None, Source::Lookup, 1_000);
        assert!(c.fresh("alpha", 1_000 + FRESH_MS - 1));
        assert!(!c.fresh("alpha", 1_000 + FRESH_MS));
    }

    #[test]
    fn the_same_answer_twice_is_not_a_change() {
        let mut c = Cache::default();
        assert!(c.remember("alpha", live(5), Source::Followed, 1_000));
        assert!(!c.remember("alpha", live(5), Source::Followed, 2_000));
        assert!(c.remember("alpha", live(6), Source::Followed, 3_000));
    }

    #[test]
    fn the_cache_stays_bounded() {
        let mut c = Cache::default();
        for i in 0..(CAP as i64 + 5) {
            c.remember(&format!("user{i}"), None, Source::Lookup, i);
        }
        assert_eq!(c.known.len(), CAP);
        assert!(!c.known.contains_key("user0"), "the oldest answer drops first");
    }
}
