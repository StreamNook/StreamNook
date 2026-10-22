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
//!
//! The same connection carries the solo player's channel while the watch
//! EventSub socket cannot subscribe (the account's socket limit is taken by
//! other devices; see `eventsub_budget`). Its raid then goes down the solo
//! path, `watch_session::on_raid`, exactly as `channel.raid` would.

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

/// The Twitch user ids whose raids this connection carries: the grid's tiles
/// and, while it stands in for the watch socket, the solo channel. The socket
/// task follows this set: it LISTENs to what was added, UNLISTENs what was
/// removed and closes when it empties.
static CHANNELS: OnceLock<watch::Sender<HashSet<String>>> = OnceLock::new();

/// Who the carried channels are for.
#[derive(Default)]
struct Wanted {
    grid: HashSet<String>,
    solo: Option<String>,
}

static WANTED: once_cell::sync::Lazy<std::sync::Mutex<Wanted>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(Wanted::default()));

fn wanted() -> std::sync::MutexGuard<'static, Wanted> {
    WANTED.lock().unwrap_or_else(|p| p.into_inner())
}

/// Where a raid out of `source_id` goes: the grid's tile card, the solo
/// player's raid follow, or both when the same channel is in each.
fn routes(wanted: &Wanted, source_id: &str) -> (bool, bool) {
    (wanted.grid.contains(source_id), wanted.solo.as_deref() == Some(source_id))
}

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
    #[serde(default)]
    viewer_count: i64,
}

/// Replace the set of channels whose raids reach the grid. Starts the socket
/// task on first use; an empty set closes the connection.
pub fn set_channels(app: &AppHandle, channel_ids: Vec<String>) {
    let grid: HashSet<String> = channel_ids
        .into_iter()
        .filter(|id| !id.is_empty())
        .take(PUBSUB_MAX_TOPICS_PER_CONNECTION - 1)
        .collect();
    wanted().grid = grid;
    carry(app);
}

/// Carry the solo player's raids on this connection (`Some`), or stop
/// (`None`: the watch socket's `channel.raid` has them).
pub fn set_solo(app: &AppHandle, channel_id: Option<String>) {
    let solo = channel_id.filter(|id| !id.is_empty());
    {
        let mut w = wanted();
        if w.solo == solo {
            return;
        }
        w.solo = solo;
    }
    carry(app);
}

/// Send the union of what the grid and the solo player want to the task.
fn carry(app: &AppHandle) {
    let wanted: HashSet<String> = {
        let w = wanted();
        w.grid.iter().cloned().chain(w.solo.clone()).collect()
    };
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
/// out right now, with the raiding party's size.
fn parse_raid_go(message: &str) -> Option<(String, TileRaid, i64)> {
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
    let viewers = raid.viewer_count;
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
        viewers,
    ))
}

/// The solo player's raid, as `channel.raid` would have delivered it. PubSub
/// names no source login; the watch session matches by id.
fn solo_raid(raid: &TileRaid, viewers: i64) -> crate::services::eventsub_service::RaidEvent {
    crate::services::eventsub_service::RaidEvent {
        from_broadcaster_user_id: raid.source_id.clone(),
        from_broadcaster_user_login: String::new(),
        from_broadcaster_user_name: String::new(),
        to_broadcaster_user_id: raid.target_id.clone(),
        to_broadcaster_user_login: raid.target_login.clone(),
        to_broadcaster_user_name: raid.target_name.clone(),
        viewers: viewers.clamp(0, i32::MAX as i64) as i32,
    }
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
                            let Some((raid_id, raid, viewers)) =
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
                            let (to_grid, to_solo) = routes(&self::wanted(), &raid.source_id);
                            if to_grid {
                                let _ = app.emit("multi-nook://raid", &raid);
                            }
                            if to_solo {
                                let event = solo_raid(&raid, viewers);
                                let _ = app.emit("eventsub://raid", &event);
                                crate::services::watch_session::on_raid(app, &event);
                            }
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
        let (id, raid, viewers) = parse_raid_go(GO).expect("raid_go_v2 parses");
        assert_eq!(id, "e54dfc27-08c4-4061-acaa-f57966991add");
        assert_eq!(viewers, 4882);
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
    fn a_solo_raid_reads_as_channel_raid() {
        let (_, raid, viewers) = parse_raid_go(GO).unwrap();
        let event = solo_raid(&raid, viewers);
        assert_eq!(event.from_broadcaster_user_id, "451786566");
        assert_eq!(event.to_broadcaster_user_login, "bigex");
        assert_eq!(event.to_broadcaster_user_name, "BigEx");
        assert_eq!(event.viewers, 4882);
    }

    #[test]
    fn a_raid_goes_where_its_channel_is_carried_for() {
        let mut w = Wanted::default();
        w.grid.insert("1".into());
        w.solo = Some("2".into());
        assert_eq!(routes(&w, "1"), (true, false), "a tile's card");
        assert_eq!(routes(&w, "2"), (false, true), "the solo player's follow");
        assert_eq!(routes(&w, "3"), (false, false));
        w.grid.insert("2".into());
        assert_eq!(routes(&w, "2"), (true, true));
    }

    #[test]
    fn the_countdown_is_not_a_raid_yet() {
        let update = GO.replace("raid_go_v2", "raid_update_v2");
        assert!(parse_raid_go(&update).is_none());
    }
}
