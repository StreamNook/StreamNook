use log::{debug, error};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Listener};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use chrono::Utc;

use crate::models::drops::{ChannelPointsClaim, ChannelPointsClaimType};
use crate::services::channel_points_websocket_service::ChannelPointsWebSocketService;
use crate::services::drops_auth_service::DropsAuthService;
use crate::services::drops_service::DropsService;

/// How often the automation balance poll re-reads followed-channel balances. The
/// plugin earns ~10 points/min passively and sweeps bonus chests every few
/// minutes, so a 3-minute cadence catches every grab with tolerable latency
/// while keeping GQL/integrity load low (this walks the full followed list).
const AUTOMATION_POLL_INTERVAL: Duration = Duration::from_secs(180);

/// After a channel-points-earned event reports a channel, the poll leaves that
/// channel alone for this long. A GQL read this close can trail the ledger
/// (measured 0.4 s behind a claim the socket had already pushed), and taking
/// that lower figure as the baseline would re-announce the rise next cycle.
/// Well under the poll interval, so the fallback still fires when the socket
/// is silent.
const POLL_REPORT_GUARD: Duration = Duration::from_secs(60);

/// A channel's last known balance and when an event last reported it. `None`
/// means only the poll's own GQL reads have.
#[derive(Clone, Copy, Debug)]
struct PollBaseline {
    balance: i32,
    reported_at: Option<Instant>,
}

/// Realtime support for the channel the user is actually watching, plus the
/// channel-points notification path. Owns the single-channel PubSub socket
/// (instant bonus-chest availability + that channel's predictions) and a
/// GQL balance-increase poll that surfaces points the Autopilot plugin collects on
/// background channels. The watched channel's own claims are notified by the
/// `claim_channel_points` command; background-collected channels are notified here.
pub struct BackgroundService {
    is_running: Arc<RwLock<bool>>,
    pub websocket_service: Arc<Mutex<ChannelPointsWebSocketService>>,
    drops_service: Arc<Mutex<DropsService>>,
    app_handle: AppHandle,
    /// The channel currently on screen, if any. The automation poll excludes it so
    /// its claims aren't double-notified (the `claim_channel_points` command
    /// already emits for the watched channel).
    watched: Arc<RwLock<Option<(String, String)>>>,
    /// Handle to the running automation balance poll, if automation is active. Tracks
    /// the Autopilot master toggle (auto_claim_channel_points): `set_automation_active`
    /// spawns it on, aborts it off. `None` means no poll is running.
    points_poll: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// The automation poll's baseline per channel. Every channel-points-earned
    /// event advances it too (the realtime socket's user topic is account-wide,
    /// so it reports background earns as well), which is what stops the poll
    /// from announcing the same rise a second time as `automation`.
    poll_baseline: Arc<RwLock<HashMap<String, PollBaseline>>>,
}

impl BackgroundService {
    pub fn new(app_handle: AppHandle, drops_service: Arc<Mutex<DropsService>>) -> Self {
        Self {
            is_running: Arc::new(RwLock::new(false)),
            websocket_service: Arc::new(Mutex::new(ChannelPointsWebSocketService::new())),
            drops_service,
            app_handle,
            watched: Arc::new(RwLock::new(None)),
            points_poll: Arc::new(Mutex::new(None)),
            poll_baseline: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Sets up the points-earned listener once. The PubSub socket itself is not
    /// connected here; it follows the watched channel via `set_watched_channel`.
    pub async fn start(&self) {
        {
            let mut is_running = self.is_running.write().await;
            if *is_running {
                debug!("Background service is already running.");
                return;
            }
            *is_running = true;
        }

        // Accumulate lifetime/history from every channel-points-earned event
        // (the watched channel's claims via claim_channel_points, and collected
        // channels via the balance poll). The single source for the lifetime
        // stats the Drops center shows.
        let drops_service_for_stats = self.drops_service.clone();
        let poll_baseline_for_stats = self.poll_baseline.clone();
        self.app_handle.listen("channel-points-earned", move |event| {
            let drops_service = drops_service_for_stats.clone();
            let poll_baseline = poll_baseline_for_stats.clone();
            tokio::spawn(async move {
                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    let channel_id = payload["channel_id"].as_str().map(|s| s.to_string());
                    let points = payload["points"].as_i64().unwrap_or(0) as i32;
                    let reason = payload["reason"].as_str().unwrap_or("watch");
                    let balance = payload["balance"].as_i64().unwrap_or(0) as i32;
                    // Whatever reported this rise, the poll's baseline moves with
                    // it so the next read has no delta to announce again.
                    if balance > 0 {
                        if let Some(cid) = channel_id.as_deref() {
                            poll_baseline.write().await.insert(
                                cid.to_string(),
                                PollBaseline {
                                    balance,
                                    reported_at: Some(Instant::now()),
                                },
                            );
                        }
                    }
                    // Prefer the login (helix lookups + leaderboard key on it),
                    // fall back to the display name.
                    let channel_name = payload["channel_login"]
                        .as_str()
                        .or_else(|| payload["channel_display_name"].as_str())
                        .unwrap_or("")
                        .to_string();

                    let ds = drops_service.lock().await;

                    // Keep the per-channel balance current for the leaderboard
                    // and the points accolades.
                    if balance > 0 {
                        if let Some(cid) = channel_id.as_deref() {
                            ds.update_channel_points_balance(cid, &channel_name, balance)
                                .await;
                        }
                    }

                    if points > 0 {
                        debug!("Channel points earned: +{} ({})", points, reason);
                        let claim = ChannelPointsClaim {
                            id: uuid::Uuid::new_v4().to_string(),
                            channel_id: channel_id.unwrap_or_default(),
                            channel_name,
                            points_earned: points,
                            claimed_at: Utc::now(),
                            claim_type: match reason {
                                "WATCH" | "watch" => ChannelPointsClaimType::Watch,
                                "CLAIM" | "claim" | "AUTOMATION" | "automation" => {
                                    ChannelPointsClaimType::Bonus
                                }
                                _ => ChannelPointsClaimType::Watch,
                            },
                        };
                        ds.add_channel_points_claim(claim).await;
                    }
                }
            });
        });
    }

    /// Point the realtime socket at the channel now on screen: subscribes its
    /// community-points (bonus-chest availability) and predictions topics.
    /// Idempotent reconnect — calling it for a new channel drops the prior one.
    pub async fn set_watched_channel(&self, channel_id: String, login: String) {
        *self.watched.write().await = Some((channel_id.clone(), login.clone()));

        let token = match DropsAuthService::get_token().await {
            Ok(t) => t,
            // No drops credential: the realtime socket can't authenticate. The
            // chest still surfaces via the frontend's 60s poll; nothing breaks.
            Err(_) => return,
        };
        let Some(user_id) = Self::fetch_user_id(&token).await else {
            return;
        };

        let mut ws = self.websocket_service.lock().await;
        ws.register_channel_mapping(&channel_id, &login, &login).await;
        if let Err(e) = ws
            .connect_to_channels(
                vec![channel_id.clone()],
                &user_id,
                &token,
                self.app_handle.clone(),
            )
            .await
        {
            error!("Failed to connect watched-channel socket: {}", e);
        }
        ws.register_active_channel(&channel_id).await;
    }

    /// Tear the realtime socket down when no channel is being watched.
    pub async fn clear_watched_channel(&self) {
        *self.watched.write().await = None;
        self.websocket_service.lock().await.disconnect_all().await;
    }

    /// Reflect the automation master toggle (auto_claim_channel_points). On, it
    /// starts a recurring GQL balance-increase poll so points the Autopilot
    /// plugin collects on background channels surface as channel-points-earned
    /// notifications (the plugin earns in its own process and has no way to emit
    /// the event itself). Off, it stops the poll. Idempotent: a no-op when the
    /// desired state already matches, so repeated drops-settings saves don't
    /// churn the task.
    pub async fn set_automation_active(&self, active: bool) {
        let mut guard = self.points_poll.lock().await;
        if active {
            if guard.is_some() {
                return;
            }
            *guard = Some(self.spawn_points_poll());
            debug!("[CP-Auto-Poll] started");
        } else if let Some(handle) = guard.take() {
            handle.abort();
            debug!("[CP-Auto-Poll] stopped");
        }
    }

    /// One poll cycle's diff. Moves `baseline` to `balances` and returns
    /// (channel_id, login, display_name, balance, delta) for every channel whose
    /// balance rose above its baseline, skipping the seeding pass and the
    /// watched channel. A channel an event reported within
    /// `POLL_REPORT_GUARD` is left untouched: that report owns the number, and
    /// a GQL read this close may trail it.
    fn rises_since(
        baseline: &mut HashMap<String, PollBaseline>,
        balances: &[(String, String, String, i32)],
        first: bool,
        watched_id: Option<&String>,
        now: Instant,
    ) -> Vec<(String, String, String, i32, i32)> {
        balances
            .iter()
            .filter_map(|(channel_id, login, display_name, balance)| {
                let prev = baseline.get(channel_id).copied();
                let recently_reported = prev
                    .and_then(|p| p.reported_at)
                    .is_some_and(|at| now.duration_since(at) < POLL_REPORT_GUARD);
                if recently_reported {
                    return None;
                }
                baseline.insert(
                    channel_id.clone(),
                    PollBaseline {
                        balance: *balance,
                        reported_at: None,
                    },
                );
                if first || watched_id == Some(channel_id) {
                    return None;
                }
                let prev = prev?;
                if *balance <= prev.balance {
                    return None;
                }
                Some((
                    channel_id.clone(),
                    login.clone(),
                    display_name.clone(),
                    *balance,
                    *balance - prev.balance,
                ))
            })
            .collect()
    }

    /// Spawn the automation balance poll: every `AUTOMATION_POLL_INTERVAL`, read every
    /// followed channel's balance via GQL and emit channel-points-earned for any
    /// channel whose balance rose since the last cycle (excluding the watched
    /// channel, which `claim_channel_points` already notifies). The first cycle
    /// only seeds the baseline so existing holdings aren't reported as earns.
    /// Rises the realtime socket already announced are not reported again: its
    /// events advance the same baseline (see `start` and `rises_since`).
    fn spawn_points_poll(&self) -> JoinHandle<()> {
        let app_handle = self.app_handle.clone();
        let drops_service = self.drops_service.clone();
        let watched = self.watched.clone();
        let baseline = self.poll_baseline.clone();

        tokio::spawn(async move {
            let mut first = true;
            let mut ticker = tokio::time::interval(AUTOMATION_POLL_INTERVAL);

            loop {
                ticker.tick().await;

                let Some(balances) = Self::fetch_all_followed_balances().await else {
                    continue;
                };

                let watched_id = watched
                    .read()
                    .await
                    .as_ref()
                    .map(|(id, _)| id.clone());

                let rises = Self::rises_since(
                    &mut *baseline.write().await,
                    &balances,
                    first,
                    watched_id.as_ref(),
                    Instant::now(),
                );

                for (channel_id, login, display_name, balance, delta) in rises {
                    {
                        let ds = drops_service.lock().await;
                        ds.update_channel_points_balance(&channel_id, &login, balance)
                            .await;
                    }

                    debug!(
                        "[CP-Auto-Poll] +{} on {} (balance {})",
                        delta, login, balance
                    );
                    let _ = app_handle.emit(
                        "channel-points-earned",
                        serde_json::json!({
                            "channel_id": channel_id,
                            "channel_login": login,
                            "channel_display_name": display_name,
                            "points": delta,
                            "reason": "automation",
                            "balance": balance,
                        }),
                    );
                }

                first = false;
            }
        })
    }

    /// Read the channel-points balance of every followed channel via the same
    /// inline ChannelPointsContext GQL query the on-demand refresh uses, batched
    /// 35 ops per request. Returns (channel_id, login, display_name, balance) for
    /// channels with a positive balance, or None if the credential/list lookup fails.
    async fn fetch_all_followed_balances() -> Option<Vec<(String, String, String, i32)>> {
        use crate::services::twitch_service::TwitchService;
        use serde_json::json;

        const CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");
        const QUERY: &str = r#"
        query ChannelPointsContext($channelLogin: String!) {
            user(login: $channelLogin) {
                channel {
                    self {
                        communityPoints {
                            balance
                        }
                    }
                }
            }
        }
        "#;

        let token = DropsAuthService::get_token().await.ok()?;
        let client = crate::services::http::client();

        // Drain the full followed list (live or offline): (login, channel_id, display_name).
        let mut channels: Vec<(String, String, String)> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            match TwitchService::get_all_followed_channels(100, cursor.clone()).await {
                Ok((page, next)) => {
                    for s in page {
                        if !s.user_login.is_empty() && !s.user_id.is_empty() {
                            // Keep the properly-cased display name so the earned-points
                            // notification matches the PubSub path (which uses it too).
                            // Falls back to the login only when the name is missing.
                            let display_name = if s.user_name.trim().is_empty() {
                                s.user_login.clone()
                            } else {
                                s.user_name
                            };
                            channels.push((s.user_login, s.user_id, display_name));
                        }
                    }
                    match next {
                        Some(c) => cursor = Some(c),
                        None => break,
                    }
                }
                Err(e) => {
                    debug!("[CP-Auto-Poll] followed-list lookup failed: {}", e);
                    return None;
                }
            }
        }

        let mut found: Vec<(String, String, String, i32)> = Vec::new();
        for chunk in
            channels.chunks(crate::services::twitch_limits::GQL_MAX_BATCHED_OPERATIONS)
        {
            let body: Vec<serde_json::Value> = chunk
                .iter()
                .map(|(login, _id, _name)| {
                    json!({
                        "operationName": "ChannelPointsContext",
                        "query": QUERY,
                        "variables": { "channelLogin": login.to_lowercase() }
                    })
                })
                .collect();

            let resp = match client
                .post("https://gql.twitch.tv/gql")
                .header("Client-Id", CLIENT_ID)
                .header("Authorization", format!("OAuth {}", token))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    error!("[CP-Auto-Poll] balance batch failed: {}", e);
                    continue;
                }
            };

            // Status first: an over-cap batch returns 400 with valid JSON,
            // so json() succeeds and the failure would pass unlogged.
            let status = resp.status();
            let parsed: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    error!("[CP-Auto-Poll] balance batch parse failed: {}", e);
                    continue;
                }
            };

            if !status.is_success() {
                error!(
                    "[CP-Auto-Poll] balance batch HTTP {} for {} operations (cap is {}): {}",
                    status,
                    chunk.len(),
                    crate::services::twitch_limits::GQL_MAX_BATCHED_OPERATIONS,
                    parsed
                );
                continue;
            }

            // Results map to requests BY INDEX, sound only while lengths
            // agree; a short array would misattribute balances.
            let Some(arr) = parsed.as_array() else {
                error!(
                    "[CP-Auto-Poll] balance batch returned a non-array body, \
                     skipping {} channels: {}",
                    chunk.len(),
                    parsed
                );
                continue;
            };
            if arr.len() != chunk.len() {
                error!(
                    "[CP-Auto-Poll] balance batch length mismatch (sent {}, got {}); \
                     refusing to map positionally",
                    chunk.len(),
                    arr.len()
                );
                continue;
            }

            for (idx, item) in arr.iter().enumerate() {
                let Some((login, channel_id, display_name)) = chunk.get(idx) else {
                    continue;
                };
                if let Some(bal) = item
                    .pointer("/data/user/channel/self/communityPoints/balance")
                    .and_then(|v| v.as_i64())
                {
                    if bal > 0 {
                        found.push((
                            channel_id.clone(),
                            login.clone(),
                            display_name.clone(),
                            bal as i32,
                        ));
                    }
                }
            }
        }

        Some(found)
    }

    /// Resolve the authenticated user's id from the drops token (needed for the
    /// user-scoped PubSub topics).
    async fn fetch_user_id(token: &str) -> Option<String> {
        let resp = crate::services::http::client()
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", token))
            .send()
            .await
            .ok()?;
        let json: serde_json::Value = resp.json().await.ok()?;
        json["user_id"].as_str().map(|s| s.to_string())
    }
}

#[cfg(test)]
mod poll_dedupe_tests {
    use super::*;

    fn row(id: &str, balance: i32) -> (String, String, String, i32) {
        (id.to_string(), format!("{id}_login"), id.to_string(), balance)
    }

    fn polled(balance: i32) -> PollBaseline {
        PollBaseline {
            balance,
            reported_at: None,
        }
    }

    fn reported(balance: i32, at: Instant) -> PollBaseline {
        PollBaseline {
            balance,
            reported_at: Some(at),
        }
    }

    #[test]
    fn seeding_pass_reports_nothing_but_fills_the_baseline() {
        let now = Instant::now();
        let mut baseline = HashMap::new();
        let rises = BackgroundService::rises_since(&mut baseline, &[row("a", 100)], true, None, now);
        assert!(rises.is_empty());
        assert_eq!(baseline["a"].balance, 100);
    }

    #[test]
    fn a_rise_since_the_last_read_is_reported_once() {
        let now = Instant::now();
        let mut baseline = HashMap::from([("a".to_string(), polled(100))]);
        let rises = BackgroundService::rises_since(&mut baseline, &[row("a", 130)], false, None, now);
        assert_eq!(rises.len(), 1);
        assert_eq!((rises[0].3, rises[0].4), (130, 30));
        let again = BackgroundService::rises_since(&mut baseline, &[row("a", 130)], false, None, now);
        assert!(again.is_empty());
    }

    #[test]
    fn a_rise_the_socket_already_reported_is_not_repeated() {
        // The socket's event moved the baseline to 130 before the read, so the
        // read is a zero delta. Well past the guard, so the poll does look.
        let now = Instant::now();
        let mut baseline = HashMap::from([(
            "a".to_string(),
            reported(130, now - POLL_REPORT_GUARD * 2),
        )]);
        let rises = BackgroundService::rises_since(&mut baseline, &[row("a", 130)], false, None, now);
        assert!(rises.is_empty());
    }

    #[test]
    fn a_stale_read_right_after_a_report_cannot_lower_the_baseline() {
        // Measured live: the socket pushed a +50 claim (36160) and a GQL read
        // 0.35 s later still returned 36110. Taking 36110 as the baseline would
        // announce the claim again next cycle as an automation earn.
        let claimed_at = Instant::now();
        let mut baseline = HashMap::from([("z".to_string(), reported(36160, claimed_at))]);
        let stale = BackgroundService::rises_since(
            &mut baseline,
            &[row("z", 36110)],
            false,
            None,
            claimed_at + Duration::from_millis(350),
        );
        assert!(stale.is_empty());
        assert_eq!(baseline["z"].balance, 36160, "the report's figure stays");

        // Next cycle the ledger has caught up: nothing to announce.
        let next = BackgroundService::rises_since(
            &mut baseline,
            &[row("z", 36160)],
            false,
            None,
            claimed_at + AUTOMATION_POLL_INTERVAL,
        );
        assert!(next.is_empty());
        assert_eq!(baseline["z"].balance, 36160);
    }

    #[test]
    fn the_poll_still_reports_a_rise_the_socket_missed() {
        // Reported long ago, then +20 arrived with no event for it (socket down
        // or a lost push): the fallback announces it once the guard has passed.
        let now = Instant::now();
        let mut baseline = HashMap::from([(
            "a".to_string(),
            reported(100, now - POLL_REPORT_GUARD * 3),
        )]);
        let rises = BackgroundService::rises_since(&mut baseline, &[row("a", 120)], false, None, now);
        assert_eq!(rises.len(), 1);
        assert_eq!(rises[0].4, 20);
    }

    #[test]
    fn watched_channel_and_decreases_are_skipped() {
        let now = Instant::now();
        let mut baseline = HashMap::from([
            ("w".to_string(), polled(100)),
            ("d".to_string(), polled(100)),
        ]);
        let watched = "w".to_string();
        let rises = BackgroundService::rises_since(
            &mut baseline,
            &[row("w", 150), row("d", 90)],
            false,
            Some(&watched),
            now,
        );
        assert!(rises.is_empty());
        assert_eq!(baseline["d"].balance, 90, "a spend lowers the baseline once the guard is clear");
    }
}
