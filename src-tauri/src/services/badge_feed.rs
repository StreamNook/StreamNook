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
use tauri::AppHandle;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::services::badge_polling_service::BadgeNotification;

const WS_URL: &str = "wss://modroom.streamnook.app/badges";
const LATEST_URL: &str = "https://modroom.streamnook.app/badges/latest.json";
/// Reconcile poll while the socket is up; latest.json is edge-cached, so cheap.
const POLL_INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Faster cadence while the socket is down and polling is the only path.
const OFFLINE_POLL_INTERVAL: Duration = Duration::from_secs(120);
const RECONNECT_BACKOFF_SECS: &[u64] = &[2, 5, 10, 30];
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// The relay auto-answers `ping`, so silence this long means a half-open
/// socket: alive locally, delivering nothing, never closing on its own.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(90);
/// Let the app shell come up before the first network call.
const STARTUP_DELAY: Duration = Duration::from_secs(2);

static SOCKET_UP: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize)]
struct Drop {
    badge: Option<BadgeNotification>,
}

#[derive(Deserialize)]
struct Frame {
    t: Option<String>,
    drops: Option<Vec<Drop>>,
    badge: Option<BadgeNotification>,
}

/// Badges carried by one relay frame, or nothing for frames that carry none
/// (the `pong` heartbeat, unknown kinds, unparseable text).
fn badges_in(text: &str) -> Vec<BadgeNotification> {
    if text == "pong" {
        return Vec::new();
    }
    let Ok(frame) = serde_json::from_str::<Frame>(text) else {
        return Vec::new();
    };
    match frame.t.as_deref() {
        Some("history") => frame
            .drops
            .unwrap_or_default()
            .into_iter()
            .filter_map(|d| d.badge)
            .collect(),
        Some("drop") => frame.badge.into_iter().collect(),
        _ => Vec::new(),
    }
}

async fn ingest(app: &AppHandle, badges: Vec<BadgeNotification>) {
    if badges.is_empty() {
        return;
    }
    if let Err(e) = crate::commands::badge_service::ingest_badge_drops(app.clone(), badges).await {
        warn!("[BadgeFeed] ingest failed: {e}");
    }
}

async fn poll_once(app: &AppHandle) {
    let client = crate::services::http::client();
    let resp = match client.get(LATEST_URL).send().await {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            debug!("[BadgeFeed] latest.json answered {}", r.status());
            return;
        }
        Err(e) => {
            debug!("[BadgeFeed] latest.json unreachable: {e}");
            return;
        }
    };
    match resp.json::<Vec<Drop>>().await {
        Ok(drops) => {
            info!("[BadgeFeed] latest.json: {} drops", drops.len());
            ingest(app, drops.into_iter().filter_map(|d| d.badge).collect()).await;
        }
        Err(e) => debug!("[BadgeFeed] latest.json unreadable: {e}"),
    }
}

/// One socket session: connect, ping on a timer, hand every frame to ingest.
/// Returns when the relay closes, errors, or goes quiet past the liveness
/// window; the caller reconnects with backoff.
async fn run_socket(app: &AppHandle) -> anyhow::Result<()> {
    let (ws, _) = connect_async(WS_URL).await?;
    let (mut tx, mut rx) = ws.split();
    SOCKET_UP.store(true, Ordering::Relaxed);
    info!("[BadgeFeed] connected to the relay");
    let mut last_inbound = Instant::now();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await; // the first tick fires immediately; skip it
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if last_inbound.elapsed() > LIVENESS_TIMEOUT {
                    anyhow::bail!("relay went quiet for {}s", LIVENESS_TIMEOUT.as_secs());
                }
                tx.send(Message::Text("ping".into())).await?;
            }
            frame = rx.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    last_inbound = Instant::now();
                    ingest(app, badges_in(&text)).await;
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
        poll_once(&poll_app).await;
        loop {
            let up = SOCKET_UP.load(Ordering::Relaxed);
            sleep(if up { POLL_INTERVAL } else { OFFLINE_POLL_INTERVAL }).await;
            poll_once(&poll_app).await;
        }
    });
    tauri::async_runtime::spawn(async move {
        sleep(STARTUP_DELAY).await;
        let mut attempt = 0usize;
        loop {
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
            let delay = RECONNECT_BACKOFF_SECS[attempt.min(RECONNECT_BACKOFF_SECS.len() - 1)];
            attempt += 1;
            sleep(Duration::from_secs(delay)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_yield_their_badges_and_nothing_else() {
        assert!(badges_in("pong").is_empty());
        assert!(badges_in("not json").is_empty());
        assert!(badges_in(r#"{"t":"other"}"#).is_empty());
        let badge = r#"{"badge_name":"Cake","badge_set_id":"cake","badge_version":"1","badge_image_url":"https://x/y.png","status":"new"}"#;
        let drop = format!(r#"{{"t":"drop","id":"cake-v1","ts":1,"badge":{badge}}}"#);
        assert_eq!(badges_in(&drop).len(), 1);
        let history = format!(r#"{{"t":"history","drops":[{{"id":"a","ts":1,"badge":{badge}}},{{"id":"b","ts":2}}]}}"#);
        let got = badges_in(&history);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].badge_set_id, "cake");
    }
}
