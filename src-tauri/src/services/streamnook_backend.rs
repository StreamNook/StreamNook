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
        .map_err(|e| format!("{name}: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("{name}: {status} {}", body.chars().take(200).collect::<String>()));
    }
    // A function that returns nothing answers 200 or 204 with an empty body;
    // that is a success, not a body to decode.
    let body = res.text().await.map_err(|e| format!("{name}: {e}"))?;
    rpc_body(&body).map_err(|e| format!("{name}: {e}"))
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
    /// A row changed. `record` is the new row (empty on a delete).
    Row { table: String, kind: String, record: Value },
    /// The socket (re)joined after having been down. Realtime does not replay
    /// what was missed, so every registry re-reads on this.
    Rejoined,
}

/// The tables whose changes the registries follow. `users` only for inserts:
/// a new member, nothing else on that table matters here.
const WATCHED: &[(&str, &str)] = &[
    ("user_cosmetics", "*"),
    ("user_cosmetic_active", "*"),
    ("atmospheres", "*"),
    ("users", "INSERT"),
    ("user_profile_prefs", "*"),
];

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
    let changes: Vec<Value> = WATCHED
        .iter()
        .map(|(table, event)| json!({ "event": event, "schema": "public", "table": table }))
        .collect();
    let r = next_ref();
    json!({
        "topic": "realtime:streamnook-registries",
        "event": "phx_join",
        "payload": {
            "config": { "broadcast": { "self": false }, "presence": { "key": "" }, "postgres_changes": changes },
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
    if msg.get("event").and_then(Value::as_str) != Some("postgres_changes") {
        return None;
    }
    let data = msg.pointer("/payload/data")?;
    Some(Change::Row {
        table: data.get("table").and_then(Value::as_str)?.to_string(),
        kind: data.get("type").and_then(Value::as_str).unwrap_or_default().to_string(),
        record: data.get("record").cloned().unwrap_or(Value::Null),
    })
}

fn joined_ok(text: &str) -> bool {
    let Ok(msg) = serde_json::from_str::<Value>(text) else { return false };
    msg.get("event").and_then(Value::as_str) == Some("phx_reply")
        && msg.get("topic").and_then(Value::as_str) == Some("realtime:streamnook-registries")
        && msg.pointer("/payload/status").and_then(Value::as_str) == Some("ok")
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
            // A phone in the background holds no socket; the rejoin on return
            // re-reads the registries (Change::Rejoined).
            if crate::services::window_visibility::phone_idle() {
                crate::services::window_visibility::until_shown().await;
                pause = Duration::from_secs(2);
            }
            match run_socket(&ws, key, &mut joined_before).await {
                Ok(()) => pause = Duration::from_secs(2),
                Err(e) => log::debug!("[StreamNookBackend] realtime: {e}"),
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_secs(120));
        }
    });
}

async fn run_socket(ws: &str, key: &str, joined_before: &mut bool) -> Result<(), String> {
    let (mut socket, _) = tokio_tungstenite::connect_async(ws).await.map_err(|e| e.to_string())?;
    socket.send(Message::Text(join_message(key).into())).await.map_err(|e| e.to_string())?;
    let mut beat = tokio::time::interval(Duration::from_secs(25));
    beat.tick().await;
    loop {
        tokio::select! {
            _ = beat.tick() => {
                if crate::services::window_visibility::phone_idle() {
                    return Ok(());
                }
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
    fn a_postgres_change_carries_its_table_kind_and_row() {
        let text = r#"{"event":"postgres_changes","topic":"realtime:streamnook-registries","ref":null,
            "payload":{"ids":[1],"data":{"table":"user_profile_prefs","type":"UPDATE","schema":"public",
            "record":{"twitch_user_id":"42","profile_theme":"aurora"},"old_record":{}}}}"#;
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
    }

    #[test]
    fn the_join_asks_for_every_watched_table() {
        let join: Value = serde_json::from_str(&join_message("k")).unwrap();
        let changes = join.pointer("/payload/config/postgres_changes").and_then(Value::as_array).unwrap();
        assert_eq!(changes.len(), WATCHED.len());
        assert!(changes.iter().any(|c| c["table"] == "users" && c["event"] == "INSERT"));
    }
}
