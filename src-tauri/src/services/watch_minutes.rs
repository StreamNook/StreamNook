//! StreamNook's own watch-time report: one wall-clock minute of live, playing
//! watching, queued and sent in batches to `POST /api/v1/watch/minutes` with
//! the account's bearer (one at a time to `/api/v1/watch/minute` where the
//! batch route is missing).
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
//! nothing. Each counted minute joins a queue that carries the member it
//! belongs to. The queue is sent when its oldest minute is five minutes old,
//! when watching stops, and when the app exits (`flush_on_exit`). Minutes that
//! fail to send (offline, 5xx, 429) stay queued and go with the next send; the
//! server skips minutes older than 25 minutes, so the queue drops those first.

use crate::services::sn_transport::{self, Host, LogThrottle};
use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use log::{debug, warn};
use once_cell::sync::Lazy;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PATH: &str = "/api/v1/watch/minute";
const BATCH_PATH: &str = "/api/v1/watch/minutes";
const TICK: Duration = Duration::from_secs(60);
/// How old the oldest queued minute gets before the queue is sent.
const FLUSH: Duration = Duration::from_secs(300);
/// The server skips minutes older than this.
const MAX_AGE: Duration = Duration::from_secs(25 * 60);
/// The most minutes one batch carries.
const MAX_PENDING: usize = 20;
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

/// A counted minute waiting to be sent, with the member it counts for.
#[derive(Clone, Debug)]
struct Queued {
    minute: DateTime<Utc>,
    member: String,
    channel: WatchedChannel,
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
    pending: VecDeque<Queued>,
    running: bool,
    /// Sends that are waiting for a retry. Logged at `warn` so a member whose
    /// minutes do not arrive has the reason in their own log, throttled so an
    /// outage of hours stays a few lines.
    deferred: LogThrottle,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

/// Held for the whole of a send, so the timer and the exit flush never send the
/// same minutes twice.
static SENDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
/// playing source by key, so the choice is stable minute to minute. A source
/// whose channel id is empty after trimming is passed over.
fn counted_channel(s: &State) -> Option<WatchedChannel> {
    let counts = |src: &&Source| src.playing && !src.channel.channel_id.trim().is_empty();
    s.sources
        .get(MAIN)
        .filter(counts)
        .or_else(|| s.sources.values().find(counts))
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
        let member = crate::services::account_store::AccountStore::primary()
            .map(|a| a.user_id)
            .filter(|id| !id.is_empty());
        let send_now = with_state(|s| {
            if let (Some(channel), Some(member)) = (counted_channel(s).and_then(queueable), member) {
                s.pending.push_back(Queued { minute, member, channel });
                while s.pending.len() > MAX_PENDING {
                    s.pending.pop_front();
                }
            }
            s.sources.is_empty() || flush_due(&s.pending, Utc::now())
        });
        if send_now {
            flush().await;
        }
        let done = with_state(|s| {
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
    }
}

/// The longest channel id the server accepts, in UTF-16 units.
const MAX_ID: usize = 64;
/// The longest login the server accepts, in UTF-16 units.
const MAX_LOGIN: usize = 64;
/// The longest display name the server accepts, in UTF-16 units.
const MAX_NAME: usize = 120;

/// The text trimmed, then cut to whole characters whose UTF-16 length together
/// stays within `limit` (the server measures a JavaScript string's length).
fn cut_utf16(text: &str, limit: usize) -> String {
    let mut used = 0;
    text.trim()
        .chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= limit
        })
        .collect()
}

/// The channel as it is queued: None when its id is empty after trimming (a tile
/// that has not resolved its channel yet), else with the id, login and name cut
/// to the lengths the server accepts.
fn queueable(mut channel: WatchedChannel) -> Option<WatchedChannel> {
    channel.channel_id = cut_utf16(&channel.channel_id, MAX_ID);
    if channel.channel_id.is_empty() {
        return None;
    }
    channel.login = cut_utf16(&channel.login, MAX_LOGIN);
    channel.name = cut_utf16(&channel.name, MAX_NAME);
    Some(channel)
}

/// The index of the entry a 400 body names as the bad one, when it is a number
/// inside the batch.
fn rejected_index(body: &str, len: usize) -> Option<usize> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let index = usize::try_from(value.get("index")?.as_u64()?).ok()?;
    (index < len).then_some(index)
}

/// The words the token lookup uses when there is no usable account: nobody
/// signed in, a refused refresh, a stored token that is gone or cannot refresh.
const NO_ACCOUNT_PHRASES: [&str; 4] = ["Not authenticated", "Please log in again", "re-link", "No stored token"];

/// Whether a send error means no StreamNook account is signed in, as opposed to
/// the token lookup failing for any other reason (a network failure, say).
fn is_no_account(error: &str) -> bool {
    error.starts_with("no_token")
        && (NO_ACCOUNT_PHRASES.iter().any(|phrase| error.contains(phrase)) || refresh_refused_for_good(error))
}

/// Whether the text says Twitch refused a token refresh with a client error
/// other than 429, which retrying will not change.
fn refresh_refused_for_good(error: &str) -> bool {
    const MARK: &str = "Twitch refused the token refresh (";
    let Some(start) = error.find(MARK) else { return false };
    let digits: String = error[start + MARK.len()..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u16>().is_ok_and(|status| (400..500).contains(&status) && status != 429)
}

/// Whether the oldest queued minute is old enough to send the queue.
fn flush_due(pending: &VecDeque<Queued>, now: DateTime<Utc>) -> bool {
    pending
        .front()
        .is_some_and(|q| now.signed_duration_since(q.minute).to_std().is_ok_and(|age| age >= FLUSH))
}

/// Removes the minutes the server would skip as too old.
fn drop_stale(pending: &mut VecDeque<Queued>, now: DateTime<Utc>) {
    pending.retain(|q| now.signed_duration_since(q.minute).to_std().map_or(true, |age| age <= MAX_AGE));
}

/// Takes the queued minutes of the member at the front, in order, at most one
/// batch of them, leaving the other members' minutes (and any beyond the batch)
/// in place.
fn take_first_member(pending: &mut VecDeque<Queued>) -> Option<(String, Vec<Queued>)> {
    let member = pending.front()?.member.clone();
    let (mut mine, rest): (Vec<Queued>, Vec<Queued>) = pending.drain(..).partition(|q| q.member == member);
    let extra = if mine.len() > MAX_PENDING { mine.split_off(MAX_PENDING) } else { Vec::new() };
    pending.extend(extra);
    pending.extend(rest);
    Some((member, mine))
}

/// Returns minutes that were not sent to the front of the queue, in order, then
/// drops the oldest until the queue is no longer than one batch.
fn put_back(pending: &mut VecDeque<Queued>, unsent: Vec<Queued>) {
    for q in unsent.into_iter().rev() {
        pending.push_front(q);
    }
    while pending.len() > MAX_PENDING {
        pending.pop_front();
    }
}

fn minute_body(q: &Queued) -> serde_json::Value {
    serde_json::json!({
        "minute": q.minute.to_rfc3339(),
        "platform": q.channel.platform,
        "channel_id": q.channel.channel_id,
        "channel_login": q.channel.login,
        "channel_name": q.channel.name,
    })
}

/// What happened to the minutes of one member.
enum Sent {
    /// Sent, or refused in a way a retry would not change.
    Done,
    /// These were not sent and should wait for the next send; why, for the log.
    Deferred(Vec<Queued>, String),
    /// No StreamNook account is signed in: nothing can be credited.
    NoToken,
}

/// Sends minutes one at a time, oldest first, stopping at the first one worth
/// retrying.
async fn send_singly(member: &str, items: Vec<Queued>) -> Sent {
    let mut items = VecDeque::from(items);
    while let Some(q) = items.front() {
        match crate::commands::streamnook_api::post_json(PATH, &minute_body(q), Some(member)).await {
            Ok(resp) if resp.ok => {
                debug!("[WatchMinutes] {} sent ({})", q.minute, resp.body);
                note_sent();
            }
            // Rate limited or a server fault: keep it for the next send.
            Ok(resp) if resp.status == 429 || resp.status >= 500 => {
                return Sent::Deferred(items.into(), format!("HTTP {}", resp.status));
            }
            // Any other refusal (stale, malformed, signed out server-side) will
            // not change on retry.
            Ok(resp) => {
                warn!("[WatchMinutes] {} refused: HTTP {} {}", q.minute, resp.status, resp.body);
            }
            Err(e) if is_no_account(&e) => return Sent::NoToken,
            Err(e) => {
                return Sent::Deferred(items.into(), e.to_string());
            }
        }
        items.pop_front();
    }
    Sent::Done
}

/// The most times one batch is sent again after the server names a bad entry.
const MAX_PASSES: usize = 20;

/// Sends one member's minutes as a single batch, falling back to one request
/// each where the batch route is not there. A 400 that names a bad entry drops
/// that entry and sends the rest again.
async fn send_batch(member: &str, mut items: Vec<Queued>) -> Sent {
    for _ in 0..MAX_PASSES {
        if items.is_empty() {
            return Sent::Done;
        }
        let body = serde_json::json!({ "minutes": items.iter().map(minute_body).collect::<Vec<_>>() });
        match crate::commands::streamnook_api::post_json(BATCH_PATH, &body, Some(member)).await {
            Ok(resp) if resp.ok => {
                debug!("[WatchMinutes] {} minute(s) sent ({})", items.len(), resp.body);
                note_sent();
                return Sent::Done;
            }
            Ok(resp) if resp.status == 404 || resp.status == 405 => return send_singly(member, items).await,
            Ok(resp) if resp.status == 429 || resp.status >= 500 => {
                return Sent::Deferred(items, format!("HTTP {}", resp.status));
            }
            Ok(resp) if resp.status == 400 => match rejected_index(&resp.body, items.len()) {
                Some(index) => {
                    warn!("[WatchMinutes] minute {} dropped: HTTP 400 {}", items[index].minute, resp.body);
                    items.remove(index);
                }
                None => {
                    warn!("[WatchMinutes] batch refused: HTTP {} {}", resp.status, resp.body);
                    return Sent::Done;
                }
            },
            Ok(resp) => {
                warn!("[WatchMinutes] batch refused: HTTP {} {}", resp.status, resp.body);
                return Sent::Done;
            }
            Err(e) if is_no_account(&e) => return Sent::NoToken,
            Err(e) => {
                return Sent::Deferred(items, e.to_string());
            }
        }
    }
    warn!("[WatchMinutes] batch still refused after {MAX_PASSES} passes; dropped");
    Sent::Done
}

/// Sends the whole queue now, member by member, whatever the age of its oldest
/// minute. Stops at the first send that has to wait.
async fn flush() {
    let _sending = SENDING.lock().await;
    with_state(|s| drop_stale(&mut s.pending, Utc::now()));
    loop {
        let Some((member, items)) = with_state(|s| take_first_member(&mut s.pending)) else {
            return;
        };
        match send_batch(&member, items).await {
            Sent::Done | Sent::NoToken => {}
            Sent::Deferred(unsent, why) => {
                with_state(|s| put_back(&mut s.pending, unsent));
                note_deferred(&why);
                return;
            }
        }
    }
}

/// Sends the queue now, for the app quitting. The receiver fires when the send
/// finished, and a send already in flight is waited for too; None when nothing
/// is queued and nothing is being sent.
pub fn flush_on_exit() -> Option<std::sync::mpsc::Receiver<()>> {
    let in_flight = SENDING.try_lock().is_err();
    if !in_flight && with_state(|s| s.pending.is_empty()) {
        return None;
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    tauri::async_runtime::spawn(async move {
        flush().await;
        let _ = done_tx.send(());
    });
    Some(done_rx)
}

/// A send that went through ends a deferred run: it is logged once, with the
/// route that carried it.
fn note_sent() {
    if let Some(run) = with_state(|s| s.deferred.recover()) {
        warn!(
            "[WatchMinutes] sending again ({} route) after {} deferred attempt{}",
            sn_transport::route(Host::Site).as_str(),
            run,
            if run == 1 { "" } else { "s" }
        );
    }
}

/// A send that will be retried on the next tick. The first of a run is logged,
/// then one line per `sn_transport::LOG_EVERY`. Never the token: `detail` is an
/// HTTP status or the transport's error, which names the URL only.
fn note_deferred(detail: &str) {
    let (held, waiting) = with_state(|s| (s.deferred.hit(Instant::now(), sn_transport::LOG_EVERY), s.pending.len()));
    if let Some(held) = held {
        warn!(
            "[WatchMinutes] {} minute{} waiting to send: {}{}",
            waiting,
            if waiting == 1 { "" } else { "s" },
            detail,
            if held == 0 { String::new() } else { format!(" ({held} more attempts since the last line)") }
        );
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

    fn queued(member: &str, minute: DateTime<Utc>) -> Queued {
        Queued { minute, member: member.into(), channel: ch("c") }
    }

    fn at(offset_min: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap() + TimeDelta::minutes(offset_min)
    }

    #[test]
    fn the_oldest_minute_decides_when_a_send_is_due() {
        let mut q = VecDeque::new();
        assert!(!flush_due(&q, at(10)));
        q.push_back(queued("1", at(0)));
        q.push_back(queued("1", at(4)));
        assert!(!flush_due(&q, at(4)));
        assert!(flush_due(&q, at(5)));
        assert!(flush_due(&q, at(9)));
    }

    #[test]
    fn minutes_older_than_the_server_accepts_are_dropped_before_a_send() {
        let mut q = VecDeque::new();
        q.push_back(queued("1", at(0)));
        q.push_back(queued("1", at(10)));
        q.push_back(queued("1", at(26)));
        drop_stale(&mut q, at(30));
        let kept: Vec<_> = q.iter().map(|x| x.minute).collect();
        assert_eq!(kept, vec![at(10), at(26)]);
    }

    #[test]
    fn two_members_minutes_are_sent_as_two_batches_with_their_own_ids() {
        let mut q = VecDeque::new();
        q.push_back(queued("a", at(0)));
        q.push_back(queued("b", at(1)));
        q.push_back(queued("a", at(2)));
        let (first, items) = take_first_member(&mut q).unwrap();
        assert_eq!(first, "a");
        assert_eq!(items.iter().map(|x| x.minute).collect::<Vec<_>>(), vec![at(0), at(2)]);
        let (second, items) = take_first_member(&mut q).unwrap();
        assert_eq!(second, "b");
        assert_eq!(items.len(), 1);
        assert!(take_first_member(&mut q).is_none());
    }

    #[test]
    fn a_deferred_batch_returns_to_the_front_in_order() {
        let mut q = VecDeque::new();
        q.push_back(queued("b", at(3)));
        put_back(&mut q, vec![queued("a", at(0)), queued("a", at(1)), queued("a", at(2))]);
        let order: Vec<_> = q.iter().map(|x| x.minute).collect();
        assert_eq!(order, vec![at(0), at(1), at(2), at(3)]);
    }

    #[test]
    fn a_put_back_queue_never_outgrows_one_batch() {
        let mut q = VecDeque::new();
        for i in 0..MAX_PENDING as i64 {
            q.push_back(queued("a", at(100 + i)));
        }
        put_back(&mut q, vec![queued("a", at(0)), queued("a", at(1)), queued("a", at(2))]);
        assert_eq!(q.len(), MAX_PENDING);
        assert_eq!(q.front().unwrap().minute, at(100));
        assert_eq!(q.back().unwrap().minute, at(100 + MAX_PENDING as i64 - 1));
    }

    #[test]
    fn one_batch_takes_at_most_the_batch_size() {
        let mut q = VecDeque::new();
        for i in 0..(MAX_PENDING as i64 + 5) {
            q.push_back(queued("a", at(i)));
        }
        q.push_back(queued("b", at(99)));
        let (member, items) = take_first_member(&mut q).unwrap();
        assert_eq!(member, "a");
        assert_eq!(items.len(), MAX_PENDING);
        assert_eq!(q.len(), 6);
        assert_eq!(q.front().unwrap().member, "a");
    }

    #[test]
    fn a_channel_without_an_id_is_not_queued_and_long_names_are_cut() {
        let mut empty = ch("x");
        empty.channel_id = String::new();
        assert!(queueable(empty).is_none());
        let mut long = ch("x");
        long.login = "l".repeat(100);
        long.name = "é".repeat(200);
        let cut = queueable(long).unwrap();
        assert_eq!(cut.login.chars().count(), MAX_LOGIN);
        assert_eq!(cut.name.chars().count(), MAX_NAME);
    }

    #[test]
    fn limits_count_utf16_units_and_a_blank_id_is_skipped() {
        let mut emoji = ch("x");
        emoji.name = "😀".repeat(70);
        let cut = queueable(emoji).unwrap();
        assert!(cut.name.encode_utf16().count() <= MAX_NAME);
        assert_eq!(cut.name.chars().count(), 60);
        let mut blank = ch("x");
        blank.channel_id = "   ".into();
        assert!(queueable(blank).is_none());
    }

    #[test]
    fn a_source_without_an_id_does_not_win_the_minute() {
        let mut s = State::default();
        let mut blank = ch("blank");
        blank.channel_id = " ".into();
        s.sources.insert(MAIN.into(), Source { channel: blank, playing: true });
        s.sources.insert("tile-b".into(), Source { channel: ch("b"), playing: true });
        assert_eq!(counted_channel(&s).unwrap().login, "b");
    }

    #[test]
    fn a_400_names_the_entry_to_drop() {
        assert_eq!(rejected_index(r#"{"error":"bad_login","index":2}"#, 5), Some(2));
        assert_eq!(rejected_index(r#"{"error":"bad_login","index":5}"#, 5), None);
        assert_eq!(rejected_index(r#"{"error":"bad_login","index":-1}"#, 5), None);
        assert_eq!(rejected_index(r#"{"error":"bad_login"}"#, 5), None);
        assert_eq!(rejected_index("not json", 5), None);
    }

    #[test]
    fn a_network_failure_during_sign_in_is_not_a_missing_account() {
        assert!(is_no_account("no_token: Not authenticated. Please log in to Twitch first."));
        assert!(is_no_account("no_token: Token expired and refresh failed (x). Please log in again."));
        assert!(is_no_account("no_token: Account 1 token expired and has no refresh token; re-link it."));
        assert!(is_no_account("no_token: No stored token for secondary account 1"));
        assert!(!is_no_account(
            "no_token: Token refresh failed with a network error; the sign-in is kept and tried again (dns error)"
        ));
        assert!(!is_no_account("no_token: connect: refused"));
        assert!(!is_no_account("network: dns error"));
    }

    #[test]
    fn a_refused_refresh_clears_unless_it_is_rate_limited_or_a_fault() {
        assert!(is_no_account("no_token: Twitch refused the token refresh (400): invalid refresh token"));
        assert!(!is_no_account("no_token: Twitch refused the token refresh (429): slow down"));
        assert!(!is_no_account("no_token: Twitch refused the token refresh (503): unavailable"));
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
