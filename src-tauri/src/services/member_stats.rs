//! The member's lifetime counters on their StreamNook profile: messages sent,
//! channel points collected and streams watched.
//!
//! Each is counted where its event happens in Rust (a chat send Twitch or a
//! provider accepted, a bonus chest collected, a stream started), so every
//! client counts the same things once. They used to be counted by the desktop
//! page, which left a member who only used the iPhone at zero.
//!
//! The write is the `increment_user_stat` function, which adds atomically.
//! Where it is not deployed, the row is read and written back, which can lose a
//! concurrent add but keeps the counter moving. Watch time is not here: the
//! watch-minute reporter (`watch_minutes`) sends that through the API.

use log::{debug, warn};
use serde_json::{json, Value};

use crate::services::streamnook_backend as backend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Adds `amount` to the signed-in member's `stat`, in the background. Nothing
/// happens without a signed-in Twitch account or a configured backend.
pub fn count(stat: Stat, amount: i64) {
    if amount <= 0 || !backend::configured() {
        return;
    }
    let Some(member) = crate::services::account_store::AccountStore::primary().map(|a| a.user_id).filter(|id| !id.is_empty())
    else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let args = json!({ "p_user_id": member, "p_stat": stat.column(), "p_amount": amount });
        match backend::rpc("increment_user_stat", args).await {
            Ok(_) => debug!("[Stats] {} +{amount}", stat.column()),
            Err(e) if rpc_missing(&e) => {
                if let Err(e) = add_by_hand(&member, stat, amount).await {
                    warn!("[Stats] {} +{amount} failed: {e}", stat.column());
                }
            }
            Err(e) => warn!("[Stats] {} +{amount} failed: {e}", stat.column()),
        }
    });
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
    fn adds_to_the_stored_row_or_starts_from_zero() {
        let row = json!({ "user_id": "1", "messages_sent": 41 });
        assert_eq!(added(Some(&row), Stat::MessagesSent, 1), 42);
        assert_eq!(added(Some(&row), Stat::StreamsWatched, 1), 1);
        assert_eq!(added(None, Stat::ChannelPointsCollected, 50), 50);
    }
}
