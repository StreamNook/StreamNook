//! Badge-drop feed client: the WebSocket to the relay
//! (modroom.streamnook.app/badges) plus the edge-cached `latest.json` poll
//! fallback and the startup catch-up read.
//!
//! Until 2026-09-07 the page held this socket (`badgeSocketService.ts`). The
//! main window is DESTROYED when the app goes live or sits in the tray, so the
//! feed died with it and a drop pushed in that state was only discovered by
//! the next window's catch-up poll. Owned by Rust the socket lives as long as
//! the app does, and every drop reaches the same `ingest_badge_drops` logic
//! (notify-vs-store, persistence) that already lived here. Transport only:
//! frames are `{"t":"history","drops":[{badge}]}` and
//! `{"t":"drop","id","ts","badge"}`; a text `ping` is answered with `pong`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use log::{debug, info, warn};
use serde::Deserialize;
use crate::rt::AppHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use std::sync::Mutex;

use crate::services::account_automations::badges::{CatchUp, Origin, ServerClock};
use crate::services::badge_polling_service::BadgeNotification;

const WS_URL: &str = "wss://modroom.streamnook.app/badges";
const LATEST_URL: &str = "https://modroom.streamnook.app/badges/latest.json";
/// Reconcile poll while the socket is up; latest.json is edge-cached, so cheap.
const POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Faster cadence while the socket is down and polling is the only path.
const OFFLINE_POLL_INTERVAL: Duration = Duration::from_secs(120);
const RECONNECT_BACKOFF_SECS: &[u64] = &[2, 5, 10, 30];
/// Socket retry gap while modroom is reachable only through the relay.
const RELAYED_RECONNECT_SECS: u64 = 5 * 60;
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// The relay auto-answers `ping`, so silence this long means a half-open
/// socket: alive locally, delivering nothing, never closing on its own.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(90);
/// Let the app shell come up before the first network call.
const STARTUP_DELAY: Duration = Duration::from_secs(2);

static SOCKET_UP: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Which frames are the catch-up (the first poll of a session, the first
/// `history` frame of a socket) and which are live.
static CATCH_UP: Mutex<CatchUp> = Mutex::new(CatchUp::new());
/// The relay's clock from the `Date` header of its answers (the latest wins).
static CLOCK: Mutex<ServerClock> = Mutex::new(ServerClock::new());

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Server time minus this device's clock, in ms (0 until a `Date` header is read).
pub(crate) fn server_offset_ms() -> i64 {
    lock(&CLOCK).offset_ms()
}

fn note_date(date: Option<&str>) {
    if let Some(date) = date {
        lock(&CLOCK).note_date(date, chrono::Utc::now().timestamp_millis());
    }
}

/// One relay drop: its id and time with the badge.
pub(crate) struct FeedDrop {
    pub id: String,
    /// The relay's time for the drop, in ms (0 when the frame carried none).
    pub ts_ms: i64,
    pub badge: BadgeNotification,
}

/// The relay's time as ms: seconds are widened, a missing or zero time is 0.
fn feed_ts_ms(ts: Option<i64>) -> i64 {
    match ts {
        Some(t) if t > 0 && t < 100_000_000_000 => t * 1000,
        Some(t) if t > 0 => t,
        _ => 0,
    }
}

fn feed_drop(id: Option<String>, ts: Option<i64>, badge: Option<BadgeNotification>) -> Option<FeedDrop> {
    let badge = badge?;
    let id = id.filter(|i| !i.is_empty()).unwrap_or_else(|| badge.feed_id());
    Some(FeedDrop { id, ts_ms: feed_ts_ms(ts), badge })
}

#[derive(Deserialize)]
struct Drop {
    id: Option<String>,
    ts: Option<i64>,
    badge: Option<BadgeNotification>,
}

#[derive(Deserialize)]
struct Frame {
    t: Option<String>,
    drops: Option<Vec<Drop>>,
    id: Option<String>,
    ts: Option<i64>,
    badge: Option<BadgeNotification>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameKind {
    History,
    Push,
}

/// The drops one relay frame carries, with its kind; `None` for frames that
/// carry none (the `pong` heartbeat, unknown kinds, unparseable text). A
/// `history` frame with no drops still answers `Some`, so it ends the catch-up.
fn frame_drops(text: &str) -> Option<(FrameKind, Vec<FeedDrop>)> {
    if text == "pong" {
        return None;
    }
    let frame = serde_json::from_str::<Frame>(text).ok()?;
    match frame.t.as_deref() {
        Some("history") => Some((
            FrameKind::History,
            frame.drops.unwrap_or_default().into_iter().filter_map(|d| feed_drop(d.id, d.ts, d.badge)).collect(),
        )),
        Some("drop") => Some((FrameKind::Push, feed_drop(frame.id, frame.ts, frame.badge).into_iter().collect())),
        _ => None,
    }
}

async fn ingest(app: &AppHandle, drops: Vec<FeedDrop>, origin: Origin, push: bool) {
    if drops.is_empty() {
        return;
    }
    if let Err(e) = crate::commands::badge_service::ingest_badges(app, drops, origin, push).await {
        warn!("[BadgeFeed] ingest failed: {e}");
    }
}

/// One read of latest.json: the drops, or `None` when it failed.
async fn fetch_latest() -> Option<Vec<FeedDrop>> {
    let client = crate::services::http::client();
    let resp = match crate::services::sn_transport::send(client.get(LATEST_URL)).await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[BadgeFeed] latest.json answered {}", r.status());
            return None;
        }
        Err(e) => {
            debug!("[BadgeFeed] latest.json unreachable: {e}");
            return None;
        }
    };
    note_date(resp.headers().get(reqwest::header::DATE).and_then(|v| v.to_str().ok()));
    match resp.json::<Vec<Drop>>().await {
        Ok(drops) => {
            info!("[BadgeFeed] latest.json: {} drops", drops.len());
            Some(drops.into_iter().filter_map(|d| feed_drop(d.id, d.ts, d.badge)).collect())
        }
        Err(e) => {
            debug!("[BadgeFeed] latest.json unreadable: {e}");
            None
        }
    }
}

/// A poll: the first one to end after a session starts is the catch-up,
/// whether it carried drops or failed.
async fn poll_once(app: &AppHandle) {
    let drops = fetch_latest().await;
    let origin = lock(&CATCH_UP).poll_landed();
    if let Some(drops) = drops {
        ingest(app, drops, origin, false).await;
    }
}

/// One socket session: connect, ping on a timer, hand every frame to ingest.
/// Returns when the relay closes, errors, or goes quiet past the liveness
/// window; the caller reconnects with backoff.
async fn run_socket(app: &AppHandle) -> anyhow::Result<()> {
    let (ws, upgrade) = connect_async(WS_URL).await?;
    note_date(upgrade.headers().get("date").and_then(|v| v.to_str().ok()));
    let (mut tx, mut rx) = ws.split();
    SOCKET_UP.store(true, Ordering::Relaxed);
    // This session's first `history` frame is the catch-up.
    lock(&CATCH_UP).socket_opened();
    info!("[BadgeFeed] connected to the relay");
    let mut last_inbound = Instant::now();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await; // the first tick fires immediately; skip it
    loop {
        tokio::select! {
            _ = ping.tick() => {
                // A phone in the background drops the socket; the catch-up poll
                // on return picks up whatever landed meanwhile.
                if crate::services::window_visibility::phone_idle() {
                    return Ok(());
                }
                if last_inbound.elapsed() > LIVENESS_TIMEOUT {
                    anyhow::bail!("relay went quiet for {}s", LIVENESS_TIMEOUT.as_secs());
                }
                tx.send(Message::Text("ping".into())).await?;
            }
            frame = rx.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    last_inbound = Instant::now();
                    if let Some((kind, drops)) = frame_drops(&text) {
                        let origin = lock(&CATCH_UP).frame(kind == FrameKind::History);
                        ingest(app, drops, origin, kind == FrameKind::Push).await;
                    }
                }
                Some(Ok(Message::Ping(payload))) => {
                    last_inbound = Instant::now();
                    tx.send(Message::Pong(payload)).await?;
                }
                Some(Ok(Message::Close(_))) | None => anyhow::bail!("relay closed the socket"),
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(_)) => last_inbound = Instant::now(),
            }
        }
    }
}

/// Start the feed: a catch-up poll for drops that landed while the app was
/// closed, the socket with reconnect backoff, and the reconcile poll whose
/// cadence follows the socket state. Idempotent. Called once from setup,
/// inside the tokio runtime Tauri set up.
pub fn start(app: AppHandle) {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let poll_app = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(STARTUP_DELAY).await;
        // Process start opens a session: its first poll is the catch-up.
        lock(&CATCH_UP).session_started();
        poll_once(&poll_app).await;
        loop {
            let up = SOCKET_UP.load(Ordering::Relaxed);
            sleep(if up { POLL_INTERVAL } else { OFFLINE_POLL_INTERVAL }).await;
            if !crate::services::window_visibility::phone_idle() {
                poll_once(&poll_app).await;
            }
        }
    });
    tauri::async_runtime::spawn(async move {
        sleep(STARTUP_DELAY).await;
        let mut attempt = 0usize;
        loop {
            if crate::services::window_visibility::phone_idle() {
                crate::services::window_visibility::until_shown().await;
                attempt = 0;
                // Back from hidden is a new session: its poll is the catch-up.
                lock(&CATCH_UP).session_started();
                poll_once(&app).await;
            }
            let started = Instant::now();
            match run_socket(&app).await {
                Ok(()) => {}
                Err(e) => debug!("[BadgeFeed] socket ended: {e}"),
            }
            SOCKET_UP.store(false, Ordering::Relaxed);
            // A session that lived a while resets the backoff, as the page did
            // on open; a failure right after connect keeps climbing.
            if started.elapsed() > LIVENESS_TIMEOUT {
                attempt = 0;
            }
            let mut delay = RECONNECT_BACKOFF_SECS[attempt.min(RECONNECT_BACKOFF_SECS.len() - 1)];
            attempt += 1;
            // The socket has no relay. While the host only answers through it,
            // the poll above carries the drops, so the socket retries rarely.
            if crate::services::sn_transport::route(crate::services::sn_transport::Host::Modroom)
                == crate::services::sn_transport::Route::Relay
            {
                delay = delay.max(RELAYED_RECONNECT_SECS);
            }
            sleep(Duration::from_secs(delay)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_yield_their_badges_and_nothing_else() {
        assert!(frame_drops("pong").is_none());
        assert!(frame_drops("not json").is_none());
        assert!(frame_drops(r#"{"t":"other"}"#).is_none());
        let badge = r#"{"badge_name":"Cake","badge_set_id":"cake","badge_version":"1","badge_image_url":"https://x/y.png","status":"new"}"#;
        let drop = format!(r#"{{"t":"drop","id":"cake-v1","ts":1700000000000,"badge":{badge}}}"#);
        let (kind, got) = frame_drops(&drop).expect("drop frame");
        assert_eq!((kind, got.len(), got[0].id.as_str(), got[0].ts_ms), (FrameKind::Push, 1, "cake-v1", 1_700_000_000_000));
        let history = format!(r#"{{"t":"history","drops":[{{"id":"a","ts":1,"badge":{badge}}},{{"id":"b","ts":2}}]}}"#);
        let (kind, got) = frame_drops(&history).expect("history frame");
        assert_eq!((kind, got.len()), (FrameKind::History, 1));
        assert_eq!((got[0].badge.badge_set_id.as_str(), got[0].id.as_str(), got[0].ts_ms), ("cake", "a", 1000));
        // A history frame with no drops still ends the catch-up.
        assert_eq!(frame_drops(r#"{"t":"history","drops":[]}"#).map(|(k, d)| (k, d.len())), Some((FrameKind::History, 0)));
        // A drop with no id falls back to the badge's own id.
        let bare = format!(r#"{{"t":"drop","badge":{badge}}}"#);
        assert_eq!(frame_drops(&bare).unwrap().1[0].id, "cake-v1");
    }

    #[test]
    fn the_relay_time_is_read_as_milliseconds() {
        assert_eq!(feed_ts_ms(Some(1_700_000_000)), 1_700_000_000_000);
        assert_eq!(feed_ts_ms(Some(1_700_000_000_000)), 1_700_000_000_000);
        assert_eq!(feed_ts_ms(Some(0)), 0);
        assert_eq!(feed_ts_ms(None), 0);
    }

    #[test]
    fn a_failed_poll_ends_the_catch_up_and_a_date_header_sets_the_clock() {
        let mut c = CatchUp::default();
        c.session_started();
        assert_eq!(c.poll_landed(), Origin::CatchUp);
        assert_eq!(c.poll_landed(), Origin::Live);
        let src = include_str!("badge_feed.rs").replace("\r\n", "\n");
        let body = &src[src.find("async fn poll_once").unwrap()..];
        assert!(body.find("fetch_latest().await").unwrap() < body.find("poll_landed()").unwrap());
        assert!(src.contains("note_date(upgrade.headers()") && src.contains("note_date(resp.headers()"));
    }
}
