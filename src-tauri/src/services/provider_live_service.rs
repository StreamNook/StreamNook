//! Who's-live polling for non-Twitch platforms.
//!
//! Deliberately a sibling of `live_notification_service` rather than an
//! extension of it: that service is Helix-typed end to end (it calls
//! `TwitchService::get_followed_streams` and traffics in `TwitchStream`).
//!
//! Sources of truth per platform come from the adapter's `SourceCaps`:
//! `native_follows` platforms are asked for their own followed-live list;
//! everything else is polled against the app-local follow list in
//! `Settings.provider_follows`, which is re-read every sweep so a follow made
//! mid-session takes effect without a restart.
//!
//! Results reach the frontend two ways: a `provider-live-update` event carrying
//! the whole per-provider snapshot (for list rendering), and, on an
//! offline -> live transition, the SAME `streamer-went-live` event the Twitch
//! path emits, so the existing notification UI works for provider channels with
//! no frontend changes. `streamer_login` carries the composite
//! `provider:channel` key so clicking a notification routes to the right
//! platform instead of a same-named Twitch channel.

use crate::models::provider_stream::ProviderStream;
use crate::models::settings::AppState;
use crate::services::live_notification_service::LiveNotification;
use crate::services::providers::registry;
use log::debug;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration, Instant};

/// Base tick. Each provider has its own cadence on top of this, so a cheap
/// batched API isn't held back by an expensive per-channel one.
const TICK: Duration = Duration::from_secs(30);

/// Per-provider poll cadence. Kick is one batched call for up to 50 channels,
/// so it can afford to be frequent; the others are per-channel and are not.
fn cadence_for(provider: &str) -> Duration {
    match provider {
        "kick" => Duration::from_secs(60),
        "tiktok" => Duration::from_secs(90),
        "youtube" => Duration::from_secs(120),
        _ => Duration::from_secs(120),
    }
}

#[derive(Default)]
struct LiveState {
    /// Composite keys currently live, for offline -> live edge detection.
    live_keys: HashSet<String>,
    /// Latest snapshot per composite key, served to the frontend on demand.
    snapshot: HashMap<String, ProviderStream>,
}

static STATE: once_cell::sync::Lazy<Arc<RwLock<LiveState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(LiveState::default())));

/// The current who's-live snapshot across every provider, for initial paint.
pub async fn snapshot() -> Vec<ProviderStream> {
    STATE.read().await.snapshot.values().cloned().collect()
}

/// Sweep ONE provider immediately, outside the poll cadence.
///
/// Connecting an account imports the follow list but says nothing about who is
/// live, and the poller runs on its own clock — 120s for YouTube. So without this
/// a fresh sign-in shows an empty Following tab for up to two minutes and looks
/// broken, which is exactly what it looks like: the app knows who you follow and
/// simply hasn't looked yet.
///
/// `notify: false` — a sign-in should paint the list, not fire a go-live toast for
/// every channel that happened to already be streaming, the same reason the
/// poller's first sweep is unprimed.
pub async fn refresh_provider(app: AppHandle, state: AppState, provider: &str) {
    let channels: Vec<String> = {
        match state.settings.lock() {
            Ok(s) => s
                .provider_follows
                .iter()
                .filter(|f| f.provider == provider)
                .map(|f| f.channel.clone())
                .collect(),
            Err(_) => return,
        }
    };
    let Some(src) = registry().await.get_source(provider) else {
        return;
    };
    let caps = src.caps();
    let rows = if caps.native_follows {
        match src.followed_live().await {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("[ProviderLive] immediate {} refresh failed: {}", provider, e);
                return;
            }
        }
    } else if channels.is_empty() {
        return;
    } else {
        match src.live_check(&channels).await {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("[ProviderLive] immediate {} refresh failed: {}", provider, e);
                return;
            }
        }
    };
    log::info!(
        "[ProviderLive] immediate {} refresh: {} live",
        provider,
        rows.iter().filter(|r| r.is_live).count()
    );
    apply(&app, provider, rows, false).await;
}

pub fn start(app: AppHandle, state: AppState) {
    // `tauri::async_runtime::spawn`, NOT `tokio::spawn`: the setup hook runs on
    // the main thread outside any runtime context, so a bare tokio spawn panics
    // with "there is no reactor running". Every other service started from
    // setup uses this same call.
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(TICK);
        let mut last_run: HashMap<String, Instant> = HashMap::new();
        // The first sweep only populates: without this every followed channel
        // that happens to be live at launch would fire a "went live" toast.
        let mut primed = false;

        loop {
            tick.tick().await;

            let reg = registry().await;
            // Due-check BEFORE the settings lock: the tick is more frequent than
            // any provider cadence, so most ticks used to clone the whole follow
            // list just to decide nothing was due.
            let any_due = reg.sources().any(|(provider, src)| {
                let caps = src.caps();
                (caps.live_check || caps.native_follows)
                    && last_run
                        .get(provider)
                        .map(|t| t.elapsed() >= cadence_for(provider))
                        .unwrap_or(true)
            });
            if !any_due {
                continue;
            }

            // Re-read the follow list every sweep so following a channel takes
            // effect immediately rather than at the next app start.
            let follows = {
                match state.settings.lock() {
                    Ok(s) => s.provider_follows.clone(),
                    Err(e) => {
                        debug!("[ProviderLive] settings lock poisoned: {}", e);
                        continue;
                    }
                }
            };

            let mut by_provider: HashMap<String, Vec<String>> = HashMap::new();
            for f in &follows {
                by_provider
                    .entry(f.provider.clone())
                    .or_default()
                    .push(f.channel.clone());
            }

            for (provider, src) in reg.sources() {
                let caps = src.caps();
                if !caps.live_check && !caps.native_follows {
                    continue;
                }
                let channels = by_provider.get(provider).cloned().unwrap_or_default();
                if channels.is_empty() && !caps.native_follows {
                    // Nothing followed here; also clear any stale rows.
                    prune_provider(provider).await;
                    continue;
                }

                let due = last_run
                    .get(provider)
                    .map(|t| t.elapsed() >= cadence_for(provider))
                    .unwrap_or(true);
                if !due {
                    continue;
                }
                last_run.insert(provider.to_string(), Instant::now());

                let rows = if caps.native_follows {
                    match src.followed_live().await {
                        Ok(rows) => rows,
                        Err(e) => {
                            log::warn!("[ProviderLive] {} followed_live failed: {}", provider, e);
                            // Falling back to per-channel checks only helps while the
                            // list is small enough to actually cover. `live_check`
                            // caps how many it will fetch per sweep, so on a large
                            // imported list it samples a fraction — and publishing
                            // that as the answer REPLACES this provider's rows,
                            // turning "we couldn't look" into "nobody is live".
                            // Keeping the previous snapshot is the honest choice.
                            const FALLBACK_COVERAGE_LIMIT: usize = 25;
                            if channels.len() > FALLBACK_COVERAGE_LIMIT {
                                log::warn!(
                                    "[ProviderLive] {} has {} followed channels, more than the \
                                     {} a fallback sweep can cover — keeping the last known list \
                                     rather than reporting a partial one as complete",
                                    provider,
                                    channels.len(),
                                    FALLBACK_COVERAGE_LIMIT,
                                );
                                continue;
                            }
                            match src.live_check(&channels).await {
                                Ok(rows) => rows,
                                Err(e) => {
                                    debug!("[ProviderLive] {} live_check failed: {}", provider, e);
                                    continue;
                                }
                            }
                        }
                    }
                } else {
                    match src.live_check(&channels).await {
                        Ok(rows) => rows,
                        Err(e) => {
                            debug!("[ProviderLive] {} live_check failed: {}", provider, e);
                            continue;
                        }
                    }
                };

                apply(&app, provider, rows, primed).await;
            }

            primed = true;
        }
    });
}

/// Fold one provider's results into the shared state, emit the list update, and
/// fire go-live notifications for channels that just came online.
async fn apply(app: &AppHandle, provider: &str, rows: Vec<ProviderStream>, notify: bool) {
    let live: Vec<ProviderStream> = rows.into_iter().filter(|r| r.is_live).collect();
    let mut fresh_live = Vec::new();
    let mut rows_changed = false;

    {
        let mut st = STATE.write().await;
        let prefix = format!("{}:", provider);
        // Full-row comparison against the outgoing snapshot: an unchanged sweep
        // (the overwhelmingly common case) skips the all-windows emit below.
        let prev: HashMap<&String, &ProviderStream> = st
            .snapshot
            .iter()
            .filter(|(_, v)| v.provider == provider)
            .collect();
        rows_changed = prev.len() != live.len()
            || live.iter().any(|r| prev.get(&r.key) != Some(&r));
        drop(prev);
        // Drop this provider's previous rows so channels that went offline (or
        // were unfollowed) disappear, leaving other providers untouched.
        st.snapshot.retain(|_, v| v.provider != provider);
        let previously: HashSet<String> = st
            .live_keys
            .iter()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        st.live_keys.retain(|k| !k.starts_with(&prefix));

        for row in &live {
            st.live_keys.insert(row.key.clone());
            st.snapshot.insert(row.key.clone(), row.clone());
            if !previously.contains(&row.key) {
                fresh_live.push(row.clone());
            }
        }
    }

    if rows_changed {
        let _ = app.emit(
            "provider-live-update",
            serde_json::json!({ "provider": provider, "streams": live }),
        );
    }

    if !notify {
        return;
    }
    for row in fresh_live {
        let _ = app.emit(
            "streamer-went-live",
            LiveNotification {
                streamer_name: row.user_name.clone(),
                // The COMPOSITE key, so the notification's click handler opens
                // this platform's channel rather than a Twitch login by the
                // same name. Display uses `streamer_name`, so nothing shows it.
                streamer_login: row.key.clone(),
                streamer_avatar: row.profile_image_url.clone(),
                game_name: Some(row.game_name.clone()).filter(|g| !g.is_empty()),
                game_image: None,
                stream_title: Some(row.title.clone()).filter(|t| !t.is_empty()),
                stream_url: row.watch_url.clone(),
                is_test: false,
                source: None,
            },
        );
    }
}

/// Forget every row for a provider (nothing followed there any more).
async fn prune_provider(provider: &str) {
    let mut st = STATE.write().await;
    st.snapshot.retain(|_, v| v.provider != provider);
    let prefix = format!("{}:", provider);
    st.live_keys.retain(|k| !k.starts_with(&prefix));
}
