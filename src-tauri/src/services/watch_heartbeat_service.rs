//! Core parity watch heartbeat.
//!
//! Reports exactly what the official web player reports: one minute-watched
//! event per minute for the single channel the user is actually watching,
//! while it is playing. The minute goes out on both watch-reporting paths,
//! because spade ingestion is split: the `sendSpadeEvents` GraphQL mutation
//! credits drop progress but not channel points, while the legacy spade
//! track endpoint still credits channel points but not drops. Reporting on
//! one path only silently stops the other kind of crediting. This is the
//! piece that makes channel points accrue and drop progress advance for the
//! on-screen stream. It never reports more than one channel, never reports
//! an off-screen channel, and never claims anything.
//!
//! The target follows the active-channel chokepoints the frontend already
//! drives (stream start, channel hot-swap, stream stop), which also track
//! the focused chat in the multi-stream grid, so grid viewing reports the
//! focused tile only. Playback state is optimistic on a new target (streams
//! autoplay) and corrected by the player's playing and pause events.

use anyhow::{anyhow, Result};
use base64::engine::general_purpose;
use base64::Engine;
use chrono::Utc;
use flate2::write::GzEncoder;
use flate2::Compression;
use log::{debug, warn};
use reqwest::Client;
use serde_json::json;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::services::drops_auth_service::DropsAuthService;

const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

/// Legacy spade ingestion endpoint. Accepts the same minute-watched event as
/// the GraphQL mutation but feeds the pipeline that credits channel points.
const SPADE_URL: &str = "https://spade.twitch.tv/track";

/// How often the broadcast id and game info are re-resolved. Streams that
/// restart get a new broadcast id; the official player re-learns it too.
const BROADCAST_REFRESH: Duration = Duration::from_secs(900);

#[derive(Clone)]
struct WatchTarget {
    channel_id: String,
    login: String,
    broadcast_id: Option<String>,
    game_id: String,
    game_name: String,
    broadcast_checked_at: Option<Instant>,
}

pub struct WatchHeartbeatService {
    client: Client,
    target: RwLock<Option<WatchTarget>>,
    playing: AtomicBool,
    /// (drops token, resolved user id). Keyed by the token so a drops re-login
    /// as a different account re-derives the id instead of serving a stale one.
    cached_user_id: RwLock<Option<(String, String)>>,
    loop_started: AtomicBool,
}

impl WatchHeartbeatService {
    pub fn new() -> Self {
        Self {
            client: crate::services::http::client().clone(),
            target: RwLock::new(None),
            playing: AtomicBool::new(false),
            cached_user_id: RwLock::new(None),
            loop_started: AtomicBool::new(false),
        }
    }

    /// Spawns the 60-second tick loop (idempotent). Ticks no-op while there
    /// is no target or playback is paused, so an idle app sends nothing.
    pub fn start(self: &Arc<Self>) {
        if self.loop_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Consume the immediate first tick: the first minute-watched
            // lands about a minute after watching starts, like the web player.
            tick.tick().await;
            loop {
                tick.tick().await;
                service.tick().await;
            }
        });
    }

    /// Points the heartbeat at the channel now on screen. Returns the
    /// previously targeted channel id (None on a fresh start, unchanged
    /// target returns its own id). Optimistically marks playback active;
    /// the player's pause event corrects that within the same tick window.
    pub async fn set_target(&self, channel_id: String, login: String) -> Option<String> {
        let mut target = self.target.write().await;
        let previous = target.as_ref().map(|t| t.channel_id.clone());
        if previous.as_deref() != Some(channel_id.as_str()) {
            *target = Some(WatchTarget {
                channel_id,
                login,
                broadcast_id: None,
                game_id: String::new(),
                game_name: String::new(),
                broadcast_checked_at: None,
            });
        }
        self.playing.store(true, Ordering::SeqCst);
        previous
    }

    /// Stops reporting entirely (stream closed). Returns the channel id that
    /// was being watched, if any.
    pub async fn clear_target(&self) -> Option<String> {
        self.playing.store(false, Ordering::SeqCst);
        self.target.write().await.take().map(|t| t.channel_id)
    }

    /// Player playback state: true on playing, false on pause. Heartbeats
    /// only happen while true.
    pub fn set_playing(&self, playing: bool) {
        self.playing.store(playing, Ordering::SeqCst);
    }

    async fn tick(&self) {
        if !self.playing.load(Ordering::SeqCst) {
            return;
        }
        let snapshot = { self.target.read().await.clone() };
        let Some(mut target) = snapshot else {
            return;
        };
        // No drops credential means no spade surface to report on; watching
        // simply earns nothing, the same as not being logged in on the web.
        let Ok(token) = DropsAuthService::get_token().await else {
            return;
        };

        // Resolve or refresh the broadcast id and game info.
        let stale = target
            .broadcast_checked_at
            .map(|at| at.elapsed() > BROADCAST_REFRESH)
            .unwrap_or(true);
        if stale {
            match self.fetch_stream_info(&target.channel_id, &token).await {
                Ok(Some((broadcast_id, game_id, game_name))) => {
                    target.broadcast_id = Some(broadcast_id);
                    target.game_id = game_id;
                    target.game_name = game_name;
                    target.broadcast_checked_at = Some(Instant::now());
                }
                Ok(None) => {
                    // Channel is not live (offline, VOD, or ended). Nothing
                    // to report; re-check on the next stale window.
                    target.broadcast_id = None;
                    target.broadcast_checked_at = Some(Instant::now());
                }
                Err(e) => {
                    // Deliberately does NOT stamp broadcast_checked_at. Doing
                    // so treats "could not ask" as "asked, and the answer is
                    // still good", which pins a stale broadcast id for another
                    // full refresh window and lets a repeatedly-failing lookup
                    // extend that window indefinitely. On a phone connection
                    // this is a likely branch, not an exotic one. Leaving the
                    // mark alone retries on the next tick instead.
                    debug!("[Heartbeat] stream info fetch failed, retrying next tick: {e}");
                }
            }
            // Write back the refreshed snapshot unless the target moved on.
            let mut current = self.target.write().await;
            match current.as_mut() {
                Some(t) if t.channel_id == target.channel_id => *t = target.clone(),
                _ => return,
            }
        }
        let Some(broadcast_id) = target.broadcast_id.clone() else {
            return;
        };

        match self
            .send_minute_watched(&target, &broadcast_id, &token)
            .await
        {
            Ok(true) => debug!(
                "[Heartbeat] minute-watched credited for {} ({})",
                target.login, target.channel_id
            ),
            Ok(false) => debug!(
                "[Heartbeat] minute-watched not credited for {}",
                target.login
            ),
            Err(e) => warn!("[Heartbeat] send failed for {}: {e}", target.login),
        }

        match self
            .send_minute_watched_legacy(&target, &broadcast_id, &token)
            .await
        {
            Ok(true) => debug!(
                "[Heartbeat] points minute-watched accepted for {} ({})",
                target.login, target.channel_id
            ),
            Ok(false) => debug!(
                "[Heartbeat] points minute-watched rejected for {}",
                target.login
            ),
            Err(e) => warn!("[Heartbeat] points send failed for {}: {e}", target.login),
        }
    }

    /// One GQL read: the live broadcast id plus game info for the payload.
    /// Returns None when the channel is not currently live.
    async fn fetch_stream_info(
        &self,
        channel_id: &str,
        token: &str,
    ) -> Result<Option<(String, String, String)>> {
        let query = r#"
        query GetStreamInfo($channelID: ID!) {
            user(id: $channelID) {
                stream {
                    id
                    game { id name }
                }
            }
        }
        "#;
        let response = self
            .client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", CLIENT_ID)
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({ "query": query, "variables": { "channelID": channel_id } }))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        let body: serde_json::Value = response.json().await?;
        let stream = &body["data"]["user"]["stream"];
        let Some(id) = stream["id"].as_str() else {
            return Ok(None);
        };
        Ok(Some((
            id.to_string(),
            stream["game"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            stream["game"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        )))
    }

    async fn user_id(&self, token: &str) -> Result<String> {
        if let Some((cached_token, id)) = self.cached_user_id.read().await.as_ref() {
            if cached_token == token {
                return Ok(id.clone());
            }
        }
        let response = self
            .client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {token}"))
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        let body: serde_json::Value = response.json().await?;
        let id = body["user_id"]
            .as_str()
            .ok_or_else(|| anyhow!("token validation returned no user id"))?
            .to_string();
        *self.cached_user_id.write().await = Some((token.to_string(), id.clone()));
        Ok(id)
    }

    /// Send a request, retrying once on a transport error. The heartbeat fires
    /// only once every 60s, so any keep-alive connection in the pool has usually
    /// been closed by the host in between; the first reuse then fails with
    /// "error sending request for url" and a retry on a fresh connection goes
    /// through. Only transport errors are retried (a real HTTP status comes back
    /// as Ok and is handled by the caller); the request is non-idempotent but a
    /// duplicate minute-watched is harmless (Twitch dedupes by client_time).
    async fn send_once_retrying<F>(make: F) -> reqwest::Result<reqwest::Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        match make().send().await {
            Ok(resp) => Ok(resp),
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                make().send().await
            }
        }
    }

    /// The watch report for the on-screen channel: one minute-watched event sent
    /// on two transports — the raw spade ingest POST first (primary), then the
    /// `sendSpadeEvents` GraphQL mutation as a backup — so drop progress advances
    /// reliably. true when either is accepted (204).
    async fn send_minute_watched(
        &self,
        target: &WatchTarget,
        broadcast_id: &str,
        token: &str,
    ) -> Result<bool> {
        let user_id = self.user_id(token).await?;
        let inner_payload = json!([{
            "event": "minute-watched",
            "properties": {
                "broadcast_id": broadcast_id,
                "channel_id": target.channel_id,
                "channel": target.login,
                "client_time": Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                "game": target.game_name,
                "game_id": target.game_id,
                "hidden": false,
                "is_live": true,
                "live": true,
                "logged_in": true,
                "minutes_logged": 1,
                "muted": false,
                "user_id": user_id
            }
        }]);
        let minified = serde_json::to_string(&inner_payload)?;

        // PRIMARY drops transport: the raw form POST straight to the spade ingest
        // endpoint (plain base64, no gzip, no Client-Integrity) — the path that
        // credits reliably. Mirrors the Autopilot plugin's watch report.
        let encoded = general_purpose::STANDARD.encode(minified.as_bytes());
        let spade_ok = match Self::send_once_retrying(|| {
            self.client
                .post(SPADE_URL)
                .form(&[("data", encoded.as_str())])
                .timeout(Duration::from_secs(15))
        })
        .await
        {
            Ok(resp) => resp.status().as_u16() == 204,
            Err(_) => false,
        };

        // BACKUP drops transport: the sendSpadeEvents GraphQL mutation (gzip+b64),
        // kept as a safety net in case the raw post ever stops crediting.
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(minified.as_bytes())?;
        let g64 = general_purpose::STANDARD.encode(gz.finish()?);
        let mutation = json!({
            "query": "\n mutation SendEvents($input: SendSpadeEventsInput!) {\n sendSpadeEvents(input: $input) {\n statusCode\n}\n}\n",
            "variables": {
                "input": { "data": g64, "repository": "twilight", "encoding": "GZIP_B64" }
            }
        });
        let gql_ok = match Self::send_once_retrying(|| {
            self.client
                .post("https://gql.twitch.tv/gql")
                .header("Client-ID", CLIENT_ID)
                .header("Authorization", format!("OAuth {token}"))
                .header("Origin", "https://www.twitch.tv")
                .header("Referer", "https://www.twitch.tv")
                .header("Accept-Language", "en-US")
                .json(&mutation)
                .timeout(Duration::from_secs(15))
        })
        .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or(json!({}));
                body["data"]["sendSpadeEvents"]["statusCode"].as_i64() == Some(204)
            }
            _ => false,
        };

        Ok(spade_ok || gql_ok)
    }

    /// The same watched minute on the legacy spade track endpoint, which is
    /// the path that credits channel points. Field set matches the payload
    /// the channel-points service has verified against this endpoint;
    /// `location` and `player` are required there and absent from the GQL
    /// event. Plain base64, form-encoded, no gzip. HTTP 204 means accepted.
    async fn send_minute_watched_legacy(
        &self,
        target: &WatchTarget,
        broadcast_id: &str,
        token: &str,
    ) -> Result<bool> {
        let user_id = self.user_id(token).await?;
        let payload = json!([{
            "event": "minute-watched",
            "properties": {
                "broadcast_id": broadcast_id,
                "channel_id": target.channel_id,
                "channel": target.login,
                "hidden": false,
                "live": true,
                "location": "channel",
                "logged_in": true,
                "muted": false,
                "player": "site",
                "user_id": user_id
            }
        }]);
        let encoded = general_purpose::STANDARD.encode(serde_json::to_string(&payload)?.as_bytes());
        let response = Self::send_once_retrying(|| {
            self.client
                .post(SPADE_URL)
                .form(&[("data", encoded.as_str())])
                .timeout(Duration::from_secs(15))
        })
        .await?;
        Ok(response.status().as_u16() == 204)
    }
}
