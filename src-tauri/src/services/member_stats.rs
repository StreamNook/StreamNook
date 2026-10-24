//! The member's lifetime counters on their StreamNook profile: messages sent,
//! channel points collected and streams watched.
//!
//! Each is counted where its event happens in Rust (a chat send Twitch or a
//! provider accepted, a bonus chest collected, a stream started), so every
//! client counts the same things once. They used to be counted by the desktop
//! page, which left a member who only used the iPhone at zero.
//!
//! Counts are summed in memory per member and stat, and sent every five minutes
//! and when the app exits (`flush_on_exit`), so a busy chat is a handful of
//! requests rather than one per message.
//!
//! The write is the `increment_user_stat` function, which adds atomically.
//! Where it is not deployed, the row is read and written back, which can lose a
//! concurrent add but keeps the counter moving. Watch time is not here: the
//! watch-minute reporter (`watch_minutes`) sends that through the API.

use log::{debug, warn};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use crate::services::recent_emotes::EmoteUse;
use crate::services::streamnook_backend as backend;

/// How long counts are held before they are sent.
const FLUSH: Duration = Duration::from_secs(300);

#[derive(Default)]
struct Pending {
    /// Counts waiting to be sent, keyed by member and stat.
    amounts: BTreeMap<(String, Stat), i64>,
    /// Emote uses waiting to be sent, keyed by member and emote id.
    emotes: BTreeMap<(String, String), EmoteUse>,
    /// Whether the timer task is alive.
    running: bool,
}

static PENDING: Mutex<Pending> = Mutex::new(Pending { amounts: BTreeMap::new(), emotes: BTreeMap::new(), running: false });

/// Held for the whole of a send, so the timer and the exit flush never send
/// the same counts twice.
static SENDING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn pending<R>(f: impl FnOnce(&mut Pending) -> R) -> R {
    let mut guard = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stat {
    MessagesSent,
    ChannelPointsCollected,
    StreamsWatched,
}

impl Stat {
    fn column(self) -> &'static str {
        match self {
            Stat::MessagesSent => "messages_sent",
            Stat::ChannelPointsCollected => "channel_points_collected",
            Stat::StreamsWatched => "streams_watched",
        }
    }
}

/// Whether an RPC error says the function is not deployed (PostgREST's
/// PGRST202, Postgres' 42883, or a 404 on the route).
fn rpc_missing(error: &str) -> bool {
    error.contains("PGRST202") || error.contains("42883") || error.contains(" 404 ")
}

/// The counter after adding `amount` to the stored row, if one was read.
fn added(row: Option<&Value>, stat: Stat, amount: i64) -> i64 {
    row.and_then(|r| r.get(stat.column())).and_then(Value::as_i64).unwrap_or(0) + amount
}

/// Whether a failed send is worth keeping for the next one: the request never
/// reached the server, or the server was busy or faulty. A timeout after the
/// request left, and any other refusal, are not (a retry could double-count or
/// would be refused again).
fn worth_retrying(error: &str) -> bool {
    let status = error
        .split_once(": ")
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .and_then(|word| word.parse::<u16>().ok())
        .filter(|code| (100..600).contains(code));
    match status {
        Some(code) => code == 429 || code >= 500,
        None => !error.contains("timed out") && !error.contains("timeout"),
    }
}

/// Sends one summed count; the error says whether it should be kept.
async fn send_one(member: &str, stat: Stat, amount: i64) -> Result<(), String> {
    let args = json!({ "p_user_id": member, "p_stat": stat.column(), "p_amount": amount });
    match backend::rpc("increment_user_stat", args).await {
        Ok(_) => {
            debug!("[Stats] {} +{amount}", stat.column());
            Ok(())
        }
        Err(e) if rpc_missing(&e) => add_by_hand(member, stat, amount).await,
        Err(e) => Err(e),
    }
}

/// Adds `uses` to the summed emote counts of `member`.
fn merge_emotes(map: &mut BTreeMap<(String, String), EmoteUse>, member: &str, uses: Vec<EmoteUse>) {
    for u in uses {
        match map.get_mut(&(member.to_string(), u.id.clone())) {
            Some(held) => held.count += u.count,
            None => {
                map.insert((member.to_string(), u.id.clone()), u);
            }
        }
    }
}

/// Sends one summed emote count; the error says whether it should be kept.
async fn send_emote(member: &str, u: &EmoteUse) -> Result<(), String> {
    let args = json!({
        "p_user_id": member,
        "p_emote_id": u.id,
        "p_emote_name": u.name,
        "p_provider": u.provider,
        "p_image_url": u.url,
        "p_amount": u.count,
    });
    backend::rpc("increment_emote_usage", args).await.map(|_| ())
}

/// Sends everything pending, once. Entries that failed in a way worth retrying
/// go back for the next send.
async fn send_pending() {
    let _sending = SENDING.lock().await;
    let (taken, taken_emotes) = pending(|p| (std::mem::take(&mut p.amounts), std::mem::take(&mut p.emotes)));
    for ((member, stat), amount) in taken {
        if let Err(e) = send_one(&member, stat, amount).await {
            warn!("[Stats] {} +{amount} failed: {e}", stat.column());
            if worth_retrying(&e) {
                pending(|p| *p.amounts.entry((member, stat)).or_insert(0) += amount);
            }
        }
    }
    for ((member, _), u) in taken_emotes {
        if let Err(e) = send_emote(&member, &u).await {
            warn!("[Stats] emote {} +{} failed: {e}", u.id, u.count);
            if worth_retrying(&e) {
                pending(|p| merge_emotes(&mut p.emotes, &member, vec![u]));
            }
        }
    }
}

async fn run() {
    loop {
        tokio::time::sleep(FLUSH).await;
        send_pending().await;
        let done = pending(|p| {
            if p.amounts.is_empty() && p.emotes.is_empty() {
                p.running = false;
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

/// Sends what is pending now, for the app quitting. The receiver fires when the
/// send finished; None when nothing is pending, which costs no request.
pub fn flush_on_exit() -> Option<std::sync::mpsc::Receiver<()>> {
    if pending(|p| p.amounts.is_empty() && p.emotes.is_empty()) {
        return None;
    }
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    tauri::async_runtime::spawn(async move {
        send_pending().await;
        let _ = done_tx.send(());
    });
    Some(done_rx)
}

/// Adds `amount` to the signed-in member's `stat`, to be sent with the next
/// batch. Nothing happens without a signed-in Twitch account or a configured
/// backend.
pub fn count(stat: Stat, amount: i64) {
    if amount <= 0 || !backend::configured() {
        return;
    }
    let Some(member) = crate::services::account_store::AccountStore::primary().map(|a| a.user_id).filter(|id| !id.is_empty())
    else {
        return;
    };
    let spawn = pending(|p| {
        *p.amounts.entry((member, stat)).or_insert(0) += amount;
        let start = !p.running;
        p.running = true;
        start
    });
    if spawn {
        tauri::async_runtime::spawn(run());
    }
}

/// Adds the emotes a message the signed-in member sent used, to be sent with
/// the next batch. Nothing happens without a signed-in Twitch account or a
/// configured backend.
pub fn count_emotes(uses: Vec<EmoteUse>) {
    if uses.is_empty() || !backend::configured() {
        return;
    }
    let Some(member) = crate::services::account_store::AccountStore::primary().map(|a| a.user_id).filter(|id| !id.is_empty())
    else {
        return;
    };
    let spawn = pending(|p| {
        merge_emotes(&mut p.emotes, &member, uses);
        let start = !p.running;
        p.running = true;
        start
    });
    if spawn {
        tauri::async_runtime::spawn(run());
    }
}

/// The stream last counted, so a reconnect or a return to the same channel is
/// not a second stream.
static LAST_STREAM: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// A stream began on screen (`key` names it: provider and channel). Counts one
/// stream watched, unless it is the stream counted last.
pub fn stream_started(key: &str) {
    {
        let mut last = LAST_STREAM.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_deref() == Some(key) {
            return;
        }
        *last = Some(key.to_string());
    }
    count(Stat::StreamsWatched, 1);
}

async fn add_by_hand(member: &str, stat: Stat, amount: i64) -> Result<(), String> {
    let rows = backend::select("user_stats", &format!("select=*&user_id=eq.{}", urlencoding::encode(member))).await?;
    let mut row = serde_json::Map::new();
    row.insert("user_id".into(), Value::from(member));
    row.insert(stat.column().into(), Value::from(added(rows.first(), stat, amount)));
    row.insert("updated_at".into(), Value::from(chrono::Utc::now().to_rfc3339()));
    backend::upsert("user_stats", "user_id", Value::Object(row)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_function_is_told_apart_from_a_refusal() {
        assert!(rpc_missing("increment_user_stat: 404 Not Found {\"code\":\"PGRST202\"}"));
        assert!(rpc_missing("increment_user_stat: 400 Bad Request {\"code\":\"42883\"}"));
        assert!(!rpc_missing("increment_user_stat: 401 Unauthorized {\"code\":\"42501\"}"));
    }

    #[test]
    fn only_a_send_that_may_not_have_landed_cleanly_is_kept() {
        assert!(worth_retrying("increment_user_stat: error sending request for url (x)"));
        assert!(worth_retrying("increment_user_stat: 503 Service Unavailable"));
        assert!(worth_retrying("increment_user_stat: 429 Too Many Requests"));
        assert!(!worth_retrying("increment_user_stat: 401 Unauthorized {}"));
        assert!(!worth_retrying("increment_user_stat: 400 Bad Request {}"));
        assert!(!worth_retrying("increment_user_stat: operation timed out"));
    }

    #[test]
    fn counts_for_one_member_and_stat_are_summed() {
        let mut p = Pending::default();
        for amount in [1, 1, 3] {
            *p.amounts.entry(("1".to_string(), Stat::MessagesSent)).or_insert(0) += amount;
        }
        *p.amounts.entry(("2".to_string(), Stat::MessagesSent)).or_insert(0) += 1;
        assert_eq!(p.amounts.len(), 2);
        assert_eq!(p.amounts[&("1".to_string(), Stat::MessagesSent)], 5);
    }

    fn kekw(count: i64) -> EmoteUse {
        EmoteUse { id: "1".into(), name: "KEKW".into(), provider: "7tv".into(), url: "u".into(), count }
    }

    #[test]
    fn two_messages_with_the_same_emote_make_one_entry_with_the_sum() {
        let mut p = Pending::default();
        merge_emotes(&mut p.emotes, "1", vec![kekw(2)]);
        merge_emotes(&mut p.emotes, "1", vec![kekw(3)]);
        merge_emotes(&mut p.emotes, "2", vec![kekw(1)]);
        assert_eq!(p.emotes.len(), 2);
        assert_eq!(p.emotes[&("1".to_string(), "1".to_string())].count, 5);
        assert_eq!(p.emotes[&("1".to_string(), "1".to_string())].name, "KEKW");
    }

    #[test]
    fn adds_to_the_stored_row_or_starts_from_zero() {
        let row = json!({ "user_id": "1", "messages_sent": 41 });
        assert_eq!(added(Some(&row), Stat::MessagesSent, 1), 42);
        assert_eq!(added(Some(&row), Stat::StreamsWatched, 1), 1);
        assert_eq!(added(None, Stat::ChannelPointsCollected, 50), 50);
    }
}
