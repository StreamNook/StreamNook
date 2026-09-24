//! Who's-live polling for non-Twitch platforms.
//!
//! Deliberately a sibling of `live_notification_service` rather than an
//! extension of it: that service is Helix-typed end to end (it calls
//! `TwitchService::get_followed_streams` and traffics in `TwitchStream`).
//!
//! Sources of truth per platform come from the adapter's `SourceCaps`:
//! `native_follows` platforms are asked for their own followed-live list;
//! everything else is polled against the app-local follow list in
//! `Settings.provider_follows`, which is re-read every look so a follow made
//! mid-session takes effect without a restart.
//!
//! Every platform has a poller of its own, on its own clock. A look that is
//! slow or failing (YouTube reads a page per channel, a TikTok signing loads a
//! page) pushes back only that platform's next look, never another's.
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
use crate::services::providers::source::StreamSource;
use log::debug;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

/// How long a platform's poller rests between looks.
///
/// A look through the app's own follow list costs a request per channel, so it
/// runs slowly. A signed-in TikTok account's followed-live list is a single
/// request, and TikTok's own page reads it once when opened and never again, so
/// how soon a go-live shows up here is set by this number alone: it runs twice
/// as often as the Twitch follow poll. Kick is one batched call either way.
fn cadence_for(provider: &str, account_list: bool) -> Duration {
    match provider {
        "kick" => Duration::from_secs(60),
        "tiktok" if account_list => Duration::from_secs(30),
        "tiktok" => Duration::from_secs(90),
        "youtube" => Duration::from_secs(120),
        _ => Duration::from_secs(120),
    }
}

/// How long an account's followed-live list stays up while reading it fails.
/// A blip leaves the list alone; a longer outage falls back to the app's own
/// follow list, which may be only part of it.
const ACCOUNT_LIST_GRACE: Duration = Duration::from_secs(300);

/// A platform that keeps failing is asked less often: every failed look in a
/// row doubles its rest, up to this.
const MAX_REST: Duration = Duration::from_secs(300);

fn rest_after(cadence: Duration, failures: u32) -> Duration {
    cadence
        .saturating_mul(1u32 << failures.min(4))
        .min(MAX_REST.max(cadence))
}

/// Which list a platform's rows were read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum List {
    /// The signed-in account's own followed-live list (`native_follows`).
    Account,
    /// The app's follow list, checked channel by channel.
    App,
}

#[derive(Default)]
struct LiveState {
    /// Composite keys currently live, for offline -> live edge detection.
    live_keys: HashSet<String>,
    /// Latest snapshot per composite key, served to the frontend on demand.
    snapshot: HashMap<String, ProviderStream>,
    /// The list each platform's rows were last read from. A read of a
    /// different list is a new baseline, not news: the first read after
    /// launch, a sign-in or sign-out, or a fallback would otherwise announce
    /// every channel that was already live.
    read_from: HashMap<String, List>,
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
/// live, and the poller runs on its own clock (120s for YouTube). So without this
/// a fresh sign-in shows an empty Following tab for up to two minutes and looks
/// broken, which is exactly what it looks like: the app knows who you follow and
/// simply hasn't looked yet.
///
/// `notify: false`: a sign-in should paint the list, not fire a go-live toast for
/// every channel that happened to already be streaming, the same reason the
/// poller's first look is a baseline.
pub async fn refresh_provider(app: AppHandle, state: AppState, provider: &str) {
    let Some(channels) = followed_channels(&state, provider) else {
        return;
    };
    let Some(src) = registry().await.get_source(provider) else {
        return;
    };
    let caps = src.caps();
    let (from, rows) = if caps.native_follows {
        match src.followed_live().await {
            Ok(rows) => (List::Account, rows),
            Err(e) => {
                log::warn!("[ProviderLive] immediate {} refresh failed: {}", provider, e);
                return;
            }
        }
    } else if channels.is_empty() {
        return;
    } else {
        match src.live_check(&channels).await {
            Ok(rows) => (List::App, rows),
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
    apply(&app, provider, from, rows, false).await;
}

pub fn start(app: AppHandle, state: AppState) {
    // `tauri::async_runtime::spawn`, NOT `tokio::spawn`: the setup hook runs on
    // the main thread outside any runtime context, so a bare tokio spawn panics
    // with "there is no reactor running". Every other service started from
    // setup uses this same call.
    tauri::async_runtime::spawn(async move {
        for (provider, src) in registry().await.sources() {
            tauri::async_runtime::spawn(poll(app.clone(), state.clone(), provider, src));
        }
    });
}

/// What one platform's poller carries from look to look.
#[derive(Default)]
struct Poller {
    /// When the account's own list was last read, for the grace a failed read gets.
    account_read_at: Option<Instant>,
    /// Failed looks in a row, for the longer rest between them.
    failures: u32,
}

/// One platform's poller: look, rest, look again. The rest comes after the
/// look, so a slow look delays only this platform's next one.
async fn poll(app: AppHandle, state: AppState, provider: &'static str, src: Arc<dyn StreamSource>) {
    let mut poller = Poller::default();
    loop {
        if let Some((from, rows)) = look(&state, provider, src.as_ref(), &mut poller).await {
            apply(&app, provider, from, rows, true).await;
        }
        let cadence = cadence_for(provider, src.caps().native_follows);
        tokio::time::sleep(rest_after(cadence, poller.failures)).await;
    }
}

/// One look at a platform. `None` leaves its rows as they are: there was
/// nothing to look at, or the look failed.
async fn look(
    state: &AppState,
    provider: &str,
    src: &dyn StreamSource,
    poller: &mut Poller,
) -> Option<(List, Vec<ProviderStream>)> {
    let caps = src.caps();
    if !caps.live_check && !caps.native_follows {
        poller.failures = 0;
        return None;
    }
    let channels = followed_channels(state, provider)?;
    if caps.native_follows {
        match src.followed_live().await {
            Ok(rows) => {
                poller.account_read_at = Some(Instant::now());
                poller.failures = 0;
                return Some((List::Account, rows));
            }
            Err(e) => {
                // Counted even when the fallback below answers: the account's
                // list is the read that failed, and it is the one retried.
                poller.failures += 1;
                log::warn!("[ProviderLive] {} followed_live failed: {}", provider, e);
                // A blip keeps the list that is up. The app's follow list is at
                // best part of the account's, so publishing it in place of a list
                // read a minute ago would clear most of it.
                if poller
                    .account_read_at
                    .is_some_and(|t| t.elapsed() < ACCOUNT_LIST_GRACE)
                {
                    return None;
                }
                // Falling back to per-channel checks only helps while the list is
                // small enough to actually cover. `live_check` caps how many it
                // will fetch per look, so on a large imported list it samples a
                // fraction, and publishing that as the answer REPLACES this
                // provider's rows, turning "we couldn't look" into "nobody is
                // live". Keeping the previous snapshot is the honest choice.
                const FALLBACK_COVERAGE_LIMIT: usize = 25;
                if channels.len() > FALLBACK_COVERAGE_LIMIT {
                    log::warn!(
                        "[ProviderLive] {} has {} followed channels, more than the \
                         {} a fallback look can cover; keeping the last known list \
                         rather than reporting a partial one as complete",
                        provider,
                        channels.len(),
                        FALLBACK_COVERAGE_LIMIT,
                    );
                    return None;
                }
            }
        }
    } else if channels.is_empty() {
        // Nothing followed here; also clear any stale rows.
        poller.failures = 0;
        prune_provider(provider).await;
        return None;
    }
    match src.live_check(&channels).await {
        Ok(rows) => {
            if !caps.native_follows {
                poller.failures = 0;
            }
            Some((List::App, rows))
        }
        Err(e) => {
            if !caps.native_follows {
                poller.failures += 1;
            }
            debug!("[ProviderLive] {} live_check failed: {}", provider, e);
            None
        }
    }
}

/// The app's own follow list for one platform.
fn followed_channels(state: &AppState, provider: &str) -> Option<Vec<String>> {
    match state.settings.lock() {
        Ok(s) => Some(
            s.provider_follows
                .iter()
                .filter(|f| f.provider == provider)
                .map(|f| f.channel.clone())
                .collect(),
        ),
        Err(e) => {
            debug!("[ProviderLive] settings lock poisoned: {}", e);
            None
        }
    }
}

/// Fold one provider's results into the shared state, emit the list update, and
/// fire go-live notifications for channels that just came online.
async fn apply(app: &AppHandle, provider: &str, from: List, rows: Vec<ProviderStream>, notify: bool) {
    let live: Vec<ProviderStream> = rows.into_iter().filter(|r| r.is_live).collect();
    let (rows_changed, fresh_live) = {
        let mut st = STATE.write().await;
        fold(&mut st, provider, from, &live, notify)
    };

    if rows_changed {
        let _ = app.emit(
            "provider-live-update",
            serde_json::json!({ "provider": provider, "streams": live }),
        );
        // Home's unified Discover list leaves out whoever this list shows.
        crate::services::home_snapshot::note_discover_inputs_changed();
    }

    if fresh_live.is_empty() {
        return;
    }
    log::info!("[ProviderLive] {}: {} went live", provider, fresh_live.len());
    for row in fresh_live {
        crate::services::live_announce::announce(
            &app,
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

/// Replace a provider's rows with `live`. Answers whether the rows changed and
/// which channels to announce as having just gone live: none unless `notify`,
/// and none when this read is a new baseline (see `LiveState::read_from`).
fn fold(
    st: &mut LiveState,
    provider: &str,
    from: List,
    live: &[ProviderStream],
    notify: bool,
) -> (bool, Vec<ProviderStream>) {
    let same_list = st.read_from.insert(provider.to_string(), from) == Some(from);
    let announce = notify && same_list;
    let prefix = format!("{}:", provider);
    // Full-row comparison against the outgoing snapshot: an unchanged look
    // (the overwhelmingly common case) skips the all-windows emit.
    let rows_changed = {
        let prev: HashMap<&String, &ProviderStream> = st
            .snapshot
            .iter()
            .filter(|(_, v)| v.provider == provider)
            .collect();
        prev.len() != live.len() || live.iter().any(|r| prev.get(&r.key) != Some(&r))
    };
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

    let mut fresh_live = Vec::new();
    for row in live {
        st.live_keys.insert(row.key.clone());
        st.snapshot.insert(row.key.clone(), row.clone());
        if announce && !previously.contains(&row.key) {
            fresh_live.push(row.clone());
        }
    }
    (rows_changed, fresh_live)
}

/// Drop a provider's live rows, for an account just signed out: liveness read
/// with a session must not outlive it. The frontend clears its own copy on the
/// same sign-out, and Home's unified Discover list, which leaves out whoever
/// these rows show, is told to rebuild.
pub async fn forget_provider(provider: &str) {
    prune_provider(provider).await;
    crate::services::home_snapshot::note_discover_inputs_changed();
}

/// Forget every row for a provider (nothing followed there any more). The next
/// read is a new baseline.
async fn prune_provider(provider: &str) {
    let mut st = STATE.write().await;
    st.snapshot.retain(|_, v| v.provider != provider);
    let prefix = format!("{}:", provider);
    st.live_keys.retain(|k| !k.starts_with(&prefix));
    st.read_from.remove(provider);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(provider: &str, channel: &str) -> ProviderStream {
        ProviderStream {
            provider: provider.into(),
            key: format!("{}:{}", provider, channel),
            id: String::new(),
            user_id: String::new(),
            user_login: channel.into(),
            user_name: channel.into(),
            title: String::new(),
            viewer_count: 0,
            game_id: String::new(),
            game_name: String::new(),
            category_thumbnail: None,
            thumbnail_url: String::new(),
            started_at: String::new(),
            profile_image_url: None,
            is_live: true,
            watch_url: String::new(),
            tags: None,
        }
    }

    fn names(rows: &[ProviderStream]) -> Vec<&str> {
        rows.iter().map(|r| r.user_login.as_str()).collect()
    }

    #[test]
    fn the_first_read_is_a_baseline_and_the_next_one_announces() {
        let mut st = LiveState::default();
        let (_, fresh) = fold(&mut st, "tiktok", List::Account, &[row("tiktok", "a")], true);
        assert!(fresh.is_empty(), "already live at launch is not news");
        let (_, fresh) = fold(
            &mut st,
            "tiktok",
            List::Account,
            &[row("tiktok", "a"), row("tiktok", "b")],
            true,
        );
        assert_eq!(names(&fresh), ["b"]);
    }

    #[test]
    fn a_read_of_another_list_is_a_baseline() {
        let mut st = LiveState::default();
        fold(&mut st, "tiktok", List::App, &[row("tiktok", "a")], true);
        // Signing in swaps the app's list for the account's, which is longer.
        let (_, fresh) = fold(
            &mut st,
            "tiktok",
            List::Account,
            &[row("tiktok", "a"), row("tiktok", "b"), row("tiktok", "c")],
            true,
        );
        assert!(fresh.is_empty(), "b and c were live before the sign-in");
        // And back again after a fallback, then recovery: no announcements.
        fold(&mut st, "tiktok", List::App, &[], true);
        let (_, fresh) = fold(
            &mut st,
            "tiktok",
            List::Account,
            &[row("tiktok", "a"), row("tiktok", "b")],
            true,
        );
        assert!(fresh.is_empty());
    }

    #[test]
    fn a_forgotten_provider_starts_a_new_baseline() {
        let mut st = LiveState::default();
        fold(&mut st, "tiktok", List::Account, &[row("tiktok", "a")], true);
        // What prune_provider does, on the state directly.
        st.snapshot.retain(|_, v| v.provider != "tiktok");
        st.live_keys.retain(|k| !k.starts_with("tiktok:"));
        st.read_from.remove("tiktok");
        // Another account signed in before a single look ran signed out.
        let (_, fresh) = fold(&mut st, "tiktok", List::Account, &[row("tiktok", "x")], true);
        assert!(fresh.is_empty());
    }

    #[test]
    fn a_refresh_paints_without_announcing() {
        let mut st = LiveState::default();
        fold(&mut st, "kick", List::App, &[], true);
        let (changed, fresh) = fold(&mut st, "kick", List::App, &[row("kick", "a")], false);
        assert!(changed);
        assert!(fresh.is_empty());
        // Still known as live afterwards: the next look does not announce it.
        let (changed, fresh) = fold(&mut st, "kick", List::App, &[row("kick", "a")], true);
        assert!(!changed);
        assert!(fresh.is_empty());
    }

    #[test]
    fn providers_do_not_touch_each_other() {
        let mut st = LiveState::default();
        fold(&mut st, "kick", List::App, &[row("kick", "a")], true);
        fold(&mut st, "tiktok", List::Account, &[row("tiktok", "a")], true);
        let (changed, _) = fold(&mut st, "kick", List::App, &[], true);
        assert!(changed);
        assert!(st.snapshot.contains_key("tiktok:a"), "kick's empty read leaves tiktok alone");
        assert_eq!(st.read_from.get("tiktok"), Some(&List::Account));
    }

    #[test]
    fn a_failing_platform_rests_longer_up_to_a_cap() {
        let c = Duration::from_secs(30);
        assert_eq!(rest_after(c, 0), c);
        assert_eq!(rest_after(c, 1), Duration::from_secs(60));
        assert_eq!(rest_after(c, 3), Duration::from_secs(240));
        assert_eq!(rest_after(c, 40), MAX_REST);
        // A cadence already past the cap is never shortened by it.
        let slow = Duration::from_secs(600);
        assert_eq!(rest_after(slow, 2), slow);
    }

    #[test]
    fn a_signed_in_tiktok_is_polled_more_often_than_a_list_of_its_channels() {
        assert!(cadence_for("tiktok", true) < cadence_for("tiktok", false));
        assert!(cadence_for("tiktok", true) <= Duration::from_secs(30));
        assert_eq!(cadence_for("kick", true), cadence_for("kick", false));
    }
}
