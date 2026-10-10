//! StreamNook's own watch-time report: one wall-clock minute of live, playing
//! watching, sent to `POST /api/v1/watch/minute` with the account's bearer.
//!
//! The server owns everything after the send. It derives the member from the
//! token, keys the minute on (member, minute) so two devices or windows in the
//! same minute count once, feeds the lifetime `hours_watched` and favourite
//! channel stats, and awards event watch milestones. This side only decides
//! whether the minute that just ended counts:
//!
//! * a live channel is on screen (a watch session that is not an offline
//!   room), on any platform, and
//! * its player reported itself playing.
//!
//! Sources are keyed so another surface can register its own without touching
//! the main player's state; any one playing source makes the minute count, and
//! the server's primary key keeps it to one.
//!
//! Lifecycle: the loop is spawned when the first source appears and exits once
//! no source is left and nothing is waiting to be sent, so a closed player costs
//! nothing. Minutes that fail to send (offline, 5xx, 429) wait in a short queue
//! and retry on the next tick; the server accepts minutes up to ten minutes old,
//! so the queue holds no more than that.

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use log::{debug, warn};
use once_cell::sync::Lazy;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

const PATH: &str = "/api/v1/watch/minute";
const TICK: Duration = Duration::from_secs(60);
/// The server refuses minutes older than ten; keep a little margin.
const MAX_PENDING: usize = 8;
/// The main player's source key.
pub const MAIN: &str = "main";
/// Prefix of a MultiNook tile's source key; the rest is the tile's slot id.
const TILE_PREFIX: &str = "tile:";

/// A MultiNook tile's source key. Slot ids are unique per tile and never reused
/// for another channel without a stop in between, so the key needs no platform.
pub fn tile_key(slot_id: &str) -> String {
    format!("{TILE_PREFIX}{slot_id}")
}

#[derive(Clone, Debug)]
pub struct WatchedChannel {
    /// "twitch", "kick", "youtube" or "tiktok".
    pub platform: String,
    pub channel_id: String,
    pub login: String,
    pub name: String,
}

#[derive(Debug)]
struct Source {
    channel: WatchedChannel,
    playing: bool,
}

#[derive(Default)]
struct State {
    sources: BTreeMap<String, Source>,
    /// Playing state reported before (or without) a source, so a player that
    /// starts before its session registers is not missed.
    playing_hint: BTreeMap<String, bool>,
    pending: VecDeque<(DateTime<Utc>, WatchedChannel)>,
    running: bool,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Registers (or replaces) the channel a source is showing; `None` removes it.
pub fn set_source(key: &str, channel: Option<WatchedChannel>) {
    let spawn = with_state(|s| {
        match channel {
            Some(channel) => {
                let playing = s.playing_hint.get(key).copied().unwrap_or(false);
                s.sources.insert(key.to_string(), Source { channel, playing });
            }
            None => {
                s.sources.remove(key);
            }
        }
        if !s.sources.is_empty() && !s.running {
            s.running = true;
            true
        } else {
            false
        }
    });
    if spawn {
        tauri::async_runtime::spawn(run());
    }
}

/// The channel a registered source is showing.
pub fn source_channel(key: &str) -> Option<WatchedChannel> {
    with_state(|s| s.sources.get(key).map(|src| src.channel.clone()))
}

/// Replaces the channel of a source that is registered, keeping its playing
/// state; does nothing for one that is not, so a late report cannot bring a
/// closed source back.
pub fn refresh_source(key: &str, channel: WatchedChannel) {
    with_state(|s| {
        if let Some(src) = s.sources.get_mut(key) {
            src.channel = channel;
        }
    });
}

/// Removes a source and its playing state together, for a source whose player
/// is gone with it: a restart under the same key starts out not playing until
/// its new player says otherwise.
pub fn remove_source(key: &str) {
    with_state(|s| {
        s.sources.remove(key);
        s.playing_hint.remove(key);
    });
}

/// Removes every MultiNook tile's source: the grid closed.
pub fn clear_tiles() {
    with_state(|s| {
        s.sources.retain(|k, _| !k.starts_with(TILE_PREFIX));
        s.playing_hint.retain(|k, _| !k.starts_with(TILE_PREFIX));
    });
}

/// The source's player started or stopped playing.
pub fn set_playing(key: &str, playing: bool) {
    with_state(|s| {
        s.playing_hint.insert(key.to_string(), playing);
        if let Some(src) = s.sources.get_mut(key) {
            src.playing = playing;
        }
    });
}

/// The channel to credit for the minute: the main player first, else the first
/// playing source by key, so the choice is stable minute to minute.
fn counted_channel(s: &State) -> Option<WatchedChannel> {
    s.sources
        .get(MAIN)
        .filter(|src| src.playing)
        .or_else(|| s.sources.values().find(|src| src.playing))
        .map(|src| src.channel.clone())
}

async fn run() {
    // The first minute is reported a full minute after watching starts.
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + TICK, TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        let minute = Utc::now()
            .duration_trunc(TimeDelta::minutes(1))
            .unwrap_or_else(|_| Utc::now());
        let done = with_state(|s| {
            if let Some(channel) = counted_channel(s) {
                s.pending.push_back((minute, channel));
                while s.pending.len() > MAX_PENDING {
                    s.pending.pop_front();
                }
            }
            if s.sources.is_empty() && s.pending.is_empty() {
                s.running = false;
                s.playing_hint.clear();
                true
            } else {
                false
            }
        });
        if done {
            return;
        }
        flush().await;
    }
}

/// Sends queued minutes oldest first, stopping at the first one worth retrying.
async fn flush() {
    loop {
        let Some((minute, channel)) = with_state(|s| s.pending.front().cloned()) else {
            return;
        };
        let body = serde_json::json!({
            "minute": minute.to_rfc3339(),
            "platform": channel.platform,
            "channel_id": channel.channel_id,
            "channel_login": channel.login,
            "channel_name": channel.name,
        });
        match crate::commands::streamnook_api::post_json(PATH, &body, None).await {
            Ok(resp) if resp.ok => {
                debug!("[WatchMinutes] {} sent ({})", minute, resp.body);
            }
            // Rate limited or a server fault: keep it for the next tick.
            Ok(resp) if resp.status == 429 || resp.status >= 500 => {
                warn!("[WatchMinutes] {} deferred: HTTP {}", minute, resp.status);
                return;
            }
            // Any other refusal (stale, malformed, signed out server-side) will
            // not change on retry.
            Ok(resp) => {
                warn!("[WatchMinutes] {} refused: HTTP {} {}", minute, resp.status, resp.body);
            }
            // No StreamNook account signed in: nothing can be credited.
            Err(e) if e.starts_with("no_token") => {
                with_state(|s| s.pending.clear());
                return;
            }
            Err(e) => {
                debug!("[WatchMinutes] {} deferred: {}", minute, e);
                return;
            }
        }
        with_state(|s| {
            s.pending.pop_front();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(login: &str) -> WatchedChannel {
        WatchedChannel {
            platform: "twitch".into(),
            channel_id: login.into(),
            login: login.into(),
            name: login.into(),
        }
    }

    #[test]
    fn only_a_playing_source_counts_and_main_wins() {
        let mut s = State::default();
        assert!(counted_channel(&s).is_none());
        s.sources.insert("tile-b".into(), Source { channel: ch("b"), playing: true });
        s.sources.insert(MAIN.into(), Source { channel: ch("main"), playing: false });
        assert_eq!(counted_channel(&s).unwrap().login, "b");
        s.sources.get_mut(MAIN).unwrap().playing = true;
        assert_eq!(counted_channel(&s).unwrap().login, "main");
        s.sources.get_mut("tile-b").unwrap().playing = false;
        s.sources.get_mut(MAIN).unwrap().playing = false;
        assert!(counted_channel(&s).is_none());
    }

    #[test]
    fn tile_keys_never_collide_with_main_and_sort_by_slot() {
        assert_ne!(tile_key("main"), MAIN);
        let mut s = State::default();
        s.sources.insert(tile_key("cell-2"), Source { channel: ch("two"), playing: true });
        s.sources.insert(tile_key("cell-1"), Source { channel: ch("one"), playing: true });
        assert_eq!(counted_channel(&s).unwrap().login, "one");
    }
}
