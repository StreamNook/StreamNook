//! StreamNook's own database, read once in Rust for every window and both phone
//! shells.
//!
//! Each webview used to build its own Supabase client from the publishable key in
//! the bundle, download the member and cosmetics registries for itself and hold
//! its own realtime socket. A native shell runs none of that TypeScript, so the
//! reads, the realtime change feed and the legacy writes live here, and the pages
//! read Rust's copy.
//!
//! Only world-readable tables are read, with the same publishable key the desktop
//! bundle already ships (it is a build-time env value, never a secret). Writes
//! that the server owns go through `commands::streamnook_api::post_json`; the anon
//! upsert below exists only for the writes whose server path still sits behind
//! the `write_via_api` switch.

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

/// PostgREST answers an unranged select with at most the project's max rows
/// (1000) and says nothing about the rest.
const PAGE: usize = 1000;

fn creds() -> Option<(&'static str, &'static str)> {
    let url = option_env!("VITE_SUPABASE_URL")?.trim_end_matches('/');
    let key = option_env!("VITE_SUPABASE_ANON_KEY")?;
    let placeholder = url.is_empty() || key.is_empty() || url.contains("your_supabase") || key.contains("your_supabase");
    (!placeholder).then_some((url, key))
}

/// Whether this build carries the database address at all.
pub fn configured() -> bool {
    creds().is_some()
}

async fn get(table: &str, query: &str) -> Result<Vec<Value>, String> {
    let (url, key) = creds().ok_or_else(|| "StreamNook backend not configured".to_string())?;
    let res = crate::services::http::client()
        .get(format!("{url}/rest/v1/{table}?{query}"))
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .map_err(|e| format!("{table}: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("{table}: {status} {}", body.chars().take(200).collect::<String>()));
    }
    res.json::<Vec<Value>>().await.map_err(|e| format!("{table}: {e}"))
}

/// One read, for small or filtered selects (`query` is a PostgREST query string).
pub async fn select(table: &str, query: &str) -> Result<Vec<Value>, String> {
    get(table, query).await
}

/// Every row of a whole-table read, a page at a time until a short page. `query`
/// must sort on a stable key so pages neither overlap nor skip. A failed page
/// fails the read: a partial registry is worse than the last whole one.
pub async fn select_all(table: &str, query: &str) -> Result<Vec<Value>, String> {
    let mut rows = Vec::new();
    for offset in (0..).step_by(PAGE) {
        let page = get(table, &format!("{query}&limit={PAGE}&offset={offset}")).await?;
        let n = page.len();
        rows.extend(page);
        if n < PAGE {
            break;
        }
    }
    Ok(rows)
}

/// A database function the anon key may call (the `security definer` ones that
/// check their own arguments), answering what it returns.
pub async fn rpc(name: &str, args: Value) -> Result<Value, String> {
    let (url, key) = creds().ok_or_else(|| "StreamNook backend not configured".to_string())?;
    let res = crate::services::http::client()
        .post(format!("{url}/rest/v1/rpc/{name}"))
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .json(&args)
        .send()
        .await
        .map_err(|e| format!("{name}: {} {e}", send_failure_prefix(e.is_connect())))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("{name}: {status} {}", body.chars().take(200).collect::<String>()));
    }
    // A function that returns nothing answers 200 or 204 with an empty body;
    // that is a success, not a body to decode.
    let body = res.text().await.map_err(|e| format!("{name}: landed: {e}"))?;
    rpc_body(&body).map_err(|e| format!("{name}: landed: {e}"))
}

/// The marker a failed send carries in its error text: `connect:` when the
/// request never left (the server cannot have acted on it), `landed:` when it
/// may have reached the server.
fn send_failure_prefix(connect_failed: bool) -> &'static str {
    if connect_failed {
        "connect:"
    } else {
        "landed:"
    }
}

fn rpc_body(body: &str) -> Result<Value, serde_json::Error> {
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(body)
}

/// The legacy direct write: an upsert under the anon key. Used only while the
/// server path for a write is switched off (`client_config.write_via_api`).
pub async fn upsert(table: &str, on_conflict: &str, row: Value) -> Result<(), String> {
    let (url, key) = creds().ok_or_else(|| "StreamNook backend not configured".to_string())?;
    let res = crate::services::http::client()
        .post(format!("{url}/rest/v1/{table}?on_conflict={on_conflict}"))
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .header("Prefer", "resolution=merge-duplicates,return=minimal")
        .json(&row)
        .send()
        .await
        .map_err(|e| format!("{table}: {e}"))?;
    let status = res.status();
    if status.is_success() {
        return Ok(());
    }
    let body = res.text().await.unwrap_or_default();
    Err(format!("{table}: {status} {}", body.chars().take(200).collect::<String>()))
}

// ---- Realtime ------------------------------------------------------------------

/// One change the realtime feed delivered, or the feed coming back after a gap.
#[derive(Clone, Debug)]
pub enum Change {
    /// A row changed. `record` is the new row, or the old row's key columns on
    /// a delete.
    Row { table: String, kind: String, record: Value },
    /// The socket (re)joined after having been down. Realtime does not replay
    /// what was missed, so every registry re-reads on this.
    Rejoined,
}

/// The tables whose broadcasts the registries act on. Broadcasts for any other
/// table are dropped. `users` only ever broadcasts inserts: a new member.
const FOLLOWED: &[&str] = &[
    "user_cosmetics",
    "user_cosmetic_active",
    "atmospheres",
    "users",
    "user_profile_prefs",
];

/// The public Broadcast topic the database publishes row changes on.
const TOPIC: &str = "streamnook-registries";

/// The last join refusal that was logged, so a retry loop warns once per message.
static LAST_REFUSAL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

static FEED: std::sync::OnceLock<broadcast::Sender<Change>> = std::sync::OnceLock::new();
static REF: AtomicU64 = AtomicU64::new(1);

fn feed() -> &'static broadcast::Sender<Change> {
    FEED.get_or_init(|| broadcast::channel(256).0)
}

/// Every change on the watched tables, for as long as the receiver lives.
pub fn subscribe() -> broadcast::Receiver<Change> {
    feed().subscribe()
}

fn next_ref() -> String {
    REF.fetch_add(1, Ordering::Relaxed).to_string()
}

fn join_message(key: &str) -> String {
    let r = next_ref();
    json!({
        "topic": format!("realtime:{TOPIC}"),
        "event": "phx_join",
        "payload": {
            "config": {
                "broadcast": { "self": false },
                "presence": { "key": "" },
                "postgres_changes": [],
                "private": false,
            },
            "access_token": key,
        },
        "ref": r,
        "join_ref": r,
    })
    .to_string()
}

/// What a socket message means to the registries, if anything.
pub(crate) fn parse_change(text: &str) -> Option<Change> {
    let msg: Value = serde_json::from_str(text).ok()?;
    if msg.get("event").and_then(Value::as_str) != Some("broadcast") {
        return None;
    }
    if msg.pointer("/payload/event").and_then(Value::as_str) != Some("change") {
        return None;
    }
    let data = msg.pointer("/payload/payload")?;
    let table = data.get("table").and_then(Value::as_str)?;
    if !FOLLOWED.contains(&table) {
        return None;
    }
    Some(Change::Row {
        table: table.to_string(),
        kind: data.get("type").and_then(Value::as_str).unwrap_or_default().to_string(),
        record: data.get("record").cloned().unwrap_or(Value::Null),
    })
}

fn joined_ok(text: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(text) else { return false };
    msg.get("event").and_then(Value::as_str) == Some("phx_reply")
        && msg.get("topic").and_then(Value::as_str) == Some(format!("realtime:{TOPIC}").as_str())
        && msg.pointer("/payload/status").and_then(Value::as_str) == Some("ok")
}

/// Whether this is the socket's answer to a heartbeat: a reply on the
/// `phoenix` topic with status `ok`. It proves the connection stayed up, so the
/// reconnect pause resets on it and not on the join reply.
fn heartbeat_ok(text: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(text) else { return false };
    msg.get("event").and_then(Value::as_str) == Some("phx_reply")
        && msg.get("topic").and_then(Value::as_str) == Some("phoenix")
        && msg.pointer("/payload/status").and_then(Value::as_str) == Some("ok")
}

/// The status and response of a reply that refused the join, if this is one.
fn join_refusal(text: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(text).ok()?;
    if msg.get("event").and_then(Value::as_str) != Some("phx_reply")
        || msg.get("topic").and_then(Value::as_str) != Some(format!("realtime:{TOPIC}").as_str())
    {
        return None;
    }
    let status = msg.pointer("/payload/status").and_then(Value::as_str)?;
    if status == "ok" {
        return None;
    }
    let response = msg.pointer("/payload/response").cloned().unwrap_or(Value::Null);
    Some(format!("{status} {response}"))
}

/// What a message says when the registry channel is gone while the socket stays
/// open: a channel error or close, or a system error, on the registry topic.
fn channel_down(text: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(text).ok()?;
    if msg.get("topic").and_then(Value::as_str) != Some(format!("realtime:{TOPIC}").as_str()) {
        return None;
    }
    let event = msg.get("event").and_then(Value::as_str)?;
    match event {
        "phx_error" | "phx_close" => Some(format!("channel down: {event}")),
        "system" if msg.pointer("/payload/status").and_then(Value::as_str) == Some("error") => {
            let detail = msg.pointer("/payload/message").and_then(Value::as_str).unwrap_or_default();
            Some(format!("channel down: system error {detail}"))
        }
        _ => None,
    }
}

/// Holds the realtime socket for the life of the process: joins, heartbeats every
/// 25 s, and reconnects with a growing pause after a drop. Started once at boot.
pub fn start_realtime() {
    let Some((url, key)) = creds() else { return };
    let ws = format!(
        "{}/realtime/v1/websocket?apikey={key}&vsn=1.0.0",
        url.replacen("https://", "wss://", 1).replacen("http://", "ws://", 1)
    );
    tauri::async_runtime::spawn(async move {
        let mut joined_before = false;
        let mut pause = Duration::from_secs(2);
        loop {
            match run_socket(&ws, key, &mut joined_before, &mut pause).await {
                Ok(()) => pause = Duration::from_secs(2),
                Err(e) => log::debug!("[StreamNookBackend] realtime: {e}"),
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_secs(120));
        }
    });
}

async fn run_socket(ws: &str, key: &str, joined_before: &mut bool, pause: &mut Duration) -> Result<(), String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(ws).await.map_err(|e| e.to_string())?;
    socket.send(Message::Text(join_message(key).into())).await.map_err(|e| e.to_string())?;
    let mut beat = tokio::time::interval(Duration::from_secs(25));
    beat.tick().await;
    loop {
        tokio::select! {
            _ = beat.tick() => {
                let msg = json!({ "topic": "phoenix", "event": "heartbeat", "payload": {}, "ref": next_ref() }).to_string();
                socket.send(Message::Text(msg.into())).await.map_err(|e| e.to_string())?;
            }
            next = socket.next() => {
                let text = match next {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Close(_))) | None => return Ok(()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.to_string()),
                };
                if joined_ok(&text) {
                    if *joined_before {
                        let _ = feed().send(Change::Rejoined);
                    }
                    *joined_before = true;
                } else if heartbeat_ok(&text) {
                    *pause = Duration::from_secs(2);
                } else if let Some(down) = channel_down(&text) {
                    return Err(down);
                } else if let Some(refusal) = join_refusal(&text) {
                    let msg = format!("join refused: {refusal}");
                    let mut last = LAST_REFUSAL.lock().unwrap_or_else(|e| e.into_inner());
                    if last.as_deref() != Some(msg.as_str()) {
                        log::warn!("[StreamNookBackend] realtime: {msg}");
                        *last = Some(msg.clone());
                    }
                    return Err(msg);
                } else if let Some(change) = parse_change(&text) {
                    let _ = feed().send(change);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_that_returns_nothing_is_a_success() {
        assert_eq!(rpc_body("").unwrap(), Value::Null);
        assert_eq!(rpc_body("  \n").unwrap(), Value::Null);
        assert_eq!(rpc_body("5").unwrap(), json!(5));
        assert!(rpc_body("{not json").is_err());
    }

    #[test]
    fn a_database_broadcast_carries_its_table_kind_and_row() {
        let text = r#"{"event":"broadcast","topic":"realtime:streamnook-registries","ref":null,
            "payload":{"event":"change","type":"broadcast","meta":{},
            "payload":{"table":"user_profile_prefs","type":"UPDATE","id":"7f0c",
            "record":{"twitch_user_id":"42","profile_theme":"aurora"}}}}"#;
        match parse_change(text) {
            Some(Change::Row { table, kind, record }) => {
                assert_eq!(table, "user_profile_prefs");
                assert_eq!(kind, "UPDATE");
                assert_eq!(record["profile_theme"], "aurora");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn replies_and_system_messages_are_not_changes() {
        assert!(parse_change(r#"{"event":"phx_reply","payload":{"status":"ok"}}"#).is_none());
        assert!(parse_change(r#"{"event":"system","payload":{"status":"ok"}}"#).is_none());
        assert!(joined_ok(r#"{"event":"phx_reply","topic":"realtime:streamnook-registries","payload":{"status":"ok","response":{}}}"#));
        assert!(!joined_ok(r#"{"event":"phx_reply","topic":"phoenix","payload":{"status":"ok"}}"#));
        assert!(heartbeat_ok(r#"{"event":"phx_reply","topic":"phoenix","payload":{"status":"ok"}}"#));
        assert!(!heartbeat_ok(r#"{"event":"phx_reply","topic":"realtime:streamnook-registries","payload":{"status":"ok"}}"#));
    }

    #[test]
    fn a_table_the_registries_do_not_follow_is_dropped() {
        let text = r#"{"event":"broadcast","topic":"realtime:streamnook-registries",
            "payload":{"event":"change","payload":{"table":"user_identity","type":"UPDATE",
            "record":{"twitch_user_id":"42"}}}}"#;
        assert!(parse_change(text).is_none());
    }

    #[test]
    fn a_postgres_changes_message_is_not_a_change() {
        let text = r#"{"event":"postgres_changes","topic":"realtime:streamnook-registries",
            "payload":{"ids":[1],"data":{"table":"user_profile_prefs","type":"UPDATE",
            "record":{"twitch_user_id":"42"}}}}"#;
        assert!(parse_change(text).is_none());
    }

    #[test]
    fn the_join_listens_to_the_broadcast_topic_only() {
        let join: Value = serde_json::from_str(&join_message("k")).unwrap();
        assert_eq!(join["topic"], "realtime:streamnook-registries");
        assert_eq!(join.pointer("/payload/config/postgres_changes"), Some(&json!([])));
        assert_eq!(join.pointer("/payload/config/private"), Some(&json!(false)));
    }

    #[test]
    fn a_failed_send_says_whether_it_may_have_landed() {
        assert_eq!(send_failure_prefix(true), "connect:");
        assert_eq!(send_failure_prefix(false), "landed:");
    }

    #[test]
    fn a_dead_registry_channel_is_told_apart_from_other_traffic() {
        let topic = "realtime:streamnook-registries";
        let error = format!(r#"{{"event":"phx_error","topic":"{topic}","payload":{{}}}}"#);
        let close = format!(r#"{{"event":"phx_close","topic":"{topic}","payload":{{}}}}"#);
        let system = format!(r#"{{"event":"system","topic":"{topic}","payload":{{"status":"error","message":"boom"}}}}"#);
        let system_ok = format!(r#"{{"event":"system","topic":"{topic}","payload":{{"status":"ok"}}}}"#);
        assert!(channel_down(&error).is_some());
        assert!(channel_down(&close).is_some());
        assert!(channel_down(&system).unwrap().contains("boom"));
        assert!(channel_down(&system_ok).is_none());
        assert!(channel_down(r#"{"event":"phx_close","topic":"phoenix","payload":{}}"#).is_none());
    }

    #[test]
    fn a_refused_join_reply_is_recognised() {
        let refused = r#"{"event":"phx_reply","topic":"realtime:streamnook-registries","payload":{"status":"error","response":{"reason":"nope"}}}"#;
        let msg = join_refusal(refused).unwrap();
        assert!(msg.starts_with("error "));
        assert!(!joined_ok(refused));
        let ok = r#"{"event":"phx_reply","topic":"realtime:streamnook-registries","payload":{"status":"ok","response":{}}}"#;
        assert!(join_refusal(ok).is_none());
        assert!(join_refusal(r#"{"event":"phx_reply","topic":"phoenix","payload":{"status":"error"}}"#).is_none());
    }
}
