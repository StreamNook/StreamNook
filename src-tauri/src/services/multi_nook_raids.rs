//! Raids out of MultiNook tiles.
//!
//! A tile whose streamer raids someone ends with its stream, and the player can
//! only report that as a network error. Twitch announces the raid on the public
//! `raid.<channel id>` PubSub topic before the stream goes down, and that topic
//! takes no token, so one anonymous connection carries it for every Twitch tile
//! in the grid. The solo player follows raids through EventSub `channel.raid`
//! instead; that transport charges a cost unit per channel under a small
//! per-user ceiling, which a grid of up to 25 tiles would exhaust.
//!
//! Only `raid_go_v2` is acted on. `raid_update_v2` repeats through the
//! countdown, and a raid can still be cancelled until it goes.

use crate::rt::AppHandle;
use crate::services::twitch_limits::{
    PUBSUB_LISTEN_TOPICS_PER_FRAME, PUBSUB_MAX_TOPICS_PER_CONNECTION,
};
use futures_util::{SinkExt, StreamExt};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::Instant;
use tauri::Emitter;
use tokio::sync::watch;
use tokio::time::{interval, sleep, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const PUBSUB_URL: &str = "wss://pubsub-edge.twitch.tv";

/// Twitch closes a connection that has not pinged in five minutes. Pinging
/// every minute, and treating an unanswered ping as a dead socket, also means
/// a connection lost silently is noticed within two minutes rather than five.
const PING_EVERY: Duration = Duration::from_secs(60);

/// The Twitch user ids of the grid's tiles. The socket task follows this set:
/// it LISTENs to what was added, UNLISTENs what was removed and closes when it
/// empties.
static CHANNELS: OnceLock<watch::Sender<HashSet<String>>> = OnceLock::new();

/// One raid out of a tile, as the tile's card draws it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TileRaid {
    /// The raiding channel, which is the tile's channel.
    pub source_id: String,
    pub target_id: String,
    pub target_login: String,
    pub target_name: String,
    pub target_image: Option<String>,
    pub target_title: Option<String>,
}

#[derive(Deserialize)]
struct RaidMessage {
    #[serde(rename = "type")]
    kind: String,
    raid: Option<RaidBody>,
}

#[derive(Deserialize)]
struct RaidBody {
    id: String,
    source_id: String,
    target_id: String,
    target_login: String,
    #[serde(default)]
    target_display_name: String,
    #[serde(default)]
    target_profile_image: String,
    #[serde(default)]
    target_stream_name: String,
}

/// Replace the set of channels whose raids reach the grid. Starts the socket
/// task on first use; an empty set closes the connection.
pub fn set_channels(app: &AppHandle, channel_ids: Vec<String>) {
    let wanted: HashSet<String> = channel_ids
        .into_iter()
        .filter(|id| !id.is_empty())
        .take(PUBSUB_MAX_TOPICS_PER_CONNECTION)
        .collect();
    let tx = CHANNELS.get_or_init(|| {
        let (tx, rx) = watch::channel(HashSet::new());
        tauri::async_runtime::spawn(run(app.clone(), rx));
        tx
    });
    tx.send_if_modified(|current| {
        if *current == wanted {
            return false;
        }
        *current = wanted;
        true
    });
}

/// The raid a PubSub `data.message` string announces, if it is a raid going
/// out right now.
fn parse_raid_go(message: &str) -> Option<(String, TileRaid)> {
    let parsed: RaidMessage = serde_json::from_str(message).ok()?;
    if parsed.kind != "raid_go_v2" {
        return None;
    }
    let raid = parsed.raid?;
    let non_empty = |s: String| (!s.trim().is_empty()).then_some(s);
    // The topic carries the 70 px avatar; the card draws it larger, and the
    // CDN serves the same image at 300 px under the same name.
    let image = non_empty(raid.target_profile_image).map(|u| u.replace("-70x70.", "-300x300."));
    let name = non_empty(raid.target_display_name).unwrap_or_else(|| raid.target_login.clone());
    Some((
        raid.id,
        TileRaid {
            source_id: raid.source_id,
            target_id: raid.target_id,
            target_login: raid.target_login,
            target_name: name,
            target_image: image,
            target_title: non_empty(raid.target_stream_name),
        },
    ))
}

async fn run(app: AppHandle, mut rx: watch::Receiver<HashSet<String>>) {
    let mut backoff = 2u64;
    loop {
        // Idle until the grid holds a Twitch tile.
        while rx.borrow_and_update().is_empty() {
            if rx.changed().await.is_err() {
                return;
            }
        }
        let started = Instant::now();
        match session(&app, &mut rx).await {
            Ok(()) => backoff = 2,
            Err(e) => {
                // A connection that lived a while was healthy; start the
                // backoff over rather than punishing it for an old failure.
                if started.elapsed() > Duration::from_secs(120) {
                    backoff = 2;
                }
                warn!(
                    "[MultiNook raids] socket ended: {}; reconnecting in {}s",
                    e, backoff
                );
                sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
            }
        }
    }
}

/// One connection's life. `Ok` when the grid emptied and the socket was closed
/// on purpose; `Err` when it dropped and should be reopened.
async fn session(app: &AppHandle, rx: &mut watch::Receiver<HashSet<String>>) -> Result<(), String> {
    let (ws, _) = connect_async(PUBSUB_URL).await.map_err(|e| e.to_string())?;
    let (mut write, mut read) = ws.split();
    let mut listening: HashSet<String> = HashSet::new();
    let mut ping = interval(PING_EVERY);
    ping.tick().await;
    let mut pong_pending = false;
    // `raid_go_v2` can repeat; a tile's card should appear once per raid.
    let mut seen: VecDeque<String> = VecDeque::new();

    loop {
        let wanted = rx.borrow_and_update().clone();
        if wanted.is_empty() {
            let _ = write.close().await;
            return Ok(());
        }
        let removed: Vec<String> = listening
            .difference(&wanted)
            .map(|id| format!("raid.{}", id))
            .collect();
        let added: Vec<String> = wanted
            .difference(&listening)
            .map(|id| format!("raid.{}", id))
            .collect();
        for (kind, topics) in [("UNLISTEN", removed), ("LISTEN", added)] {
            for batch in topics.chunks(PUBSUB_LISTEN_TOPICS_PER_FRAME) {
                let frame = serde_json::json!({
                    "type": kind,
                    "nonce": uuid::Uuid::new_v4().to_string(),
                    "data": { "topics": batch },
                });
                write
                    .send(Message::text(frame.to_string()))
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        listening = wanted;

        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
            _ = ping.tick() => {
                if pong_pending {
                    return Err("no PONG".into());
                }
                write
                    .send(Message::text(r#"{"type":"PING"}"#))
                    .await
                    .map_err(|e| e.to_string())?;
                pong_pending = true;
            }
            msg = read.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
                        continue;
                    };
                    match frame["type"].as_str() {
                        Some("PONG") => pong_pending = false,
                        Some("RECONNECT") => return Err("server asked to reconnect".into()),
                        Some("RESPONSE") => {
                            let error = frame["error"].as_str().unwrap_or_default();
                            if !error.is_empty() {
                                warn!("[MultiNook raids] LISTEN refused: {}", error);
                            }
                        }
                        Some("MESSAGE") => {
                            let Some((raid_id, raid)) =
                                frame["data"]["message"].as_str().and_then(parse_raid_go)
                            else {
                                continue;
                            };
                            if seen.contains(&raid_id) {
                                continue;
                            }
                            seen.push_back(raid_id);
                            if seen.len() > 32 {
                                seen.pop_front();
                            }
                            debug!(
                                "[MultiNook raids] {} raided {}",
                                raid.source_id, raid.target_login
                            );
                            let _ = app.emit("multi-nook://raid", &raid);
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => return Err("closed by server".into()),
                Some(Err(e)) => return Err(e.to_string()),
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real `raid_go_v2` frame's message, captured from pubsub-edge.
    const GO: &str = r#"{"type":"raid_go_v2","raid":{"id":"e54dfc27-08c4-4061-acaa-f57966991add","creator_id":"451786566","source_id":"451786566","source_profile_image":"https://static-cdn.jtvnw.net/jtv_user_pictures/1ac703ee-57aa-4e68-86df-838d52a5193f-profile_image-70x70.png","source_stream_category":"just chatting","target_id":"415249792","target_login":"bigex","target_display_name":"BigEx","target_profile_image":"https://static-cdn.jtvnw.net/jtv_user_pictures/6316a3a7-4c01-477d-bb44-178935723cad-profile_image-70x70.png","target_stream_name":"FINALLY HERE EARLY","target_viewer_count":"953","target_stream_category":"just chatting","target_stream_thumbnail":"https://static-cdn.jtvnw.net/previews-ttv/live_user_bigex-{width}x{height}.jpg","transition_jitter_seconds":9,"force_raid_now_seconds":90,"viewer_count":4882,"raid_created_at":1790656765}}"#;

    #[test]
    fn a_raid_going_out_becomes_a_tile_raid() {
        let (id, raid) = parse_raid_go(GO).expect("raid_go_v2 parses");
        assert_eq!(id, "e54dfc27-08c4-4061-acaa-f57966991add");
        assert_eq!(
            raid,
            TileRaid {
                source_id: "451786566".into(),
                target_id: "415249792".into(),
                target_login: "bigex".into(),
                target_name: "BigEx".into(),
                target_image: Some("https://static-cdn.jtvnw.net/jtv_user_pictures/6316a3a7-4c01-477d-bb44-178935723cad-profile_image-300x300.png".into()),
                target_title: Some("FINALLY HERE EARLY".into()),
            }
        );
    }

    #[test]
    fn the_countdown_is_not_a_raid_yet() {
        let update = GO.replace("raid_go_v2", "raid_update_v2");
        assert!(parse_raid_go(&update).is_none());
    }
}
