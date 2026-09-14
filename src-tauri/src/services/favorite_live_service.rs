//! Who's-live polling for FAVOURITED channels.
//!
//! A sibling of `provider_live_service`, not an extension of it, and the split
//! is deliberate: that service answers "who that you FOLLOW is live", and its
//! `provider-live-update` event feeds the Following list and the sidebar's
//! Followed section. Folding favourites into it would quietly relabel a channel
//! you merely favourited as one you follow.
//!
//! It is also not part of `live_notification_service`, which is gated on
//! `live_notifications.enabled` — putting favourites there would mean silencing
//! notifications also emptied the sidebar's Favourites section.
//!
//! What makes this necessary at all: Twitch liveness comes from
//! `get_followed_streams` and provider liveness from `Settings.provider_follows`.
//! A channel in neither list is invisible to both, which is exactly what a
//! favourite-without-a-follow is.
//!
//! Coverage, and its limits:
//!   twitch  60s   one Helix call per 100 ids. Needs a user token; there is no
//!                 client-credentials path (see `TwitchService::get_token`), so
//!                 signed out this simply doesn't run.
//!   kick    60s   one batched `live_check` per 50 slugs.
//!   youtube 180s  per-channel page fetches, hard-capped per sweep. See below.
//!   tiktok  never TikTok registers a chat/playback provider but no
//!                 `StreamSource`, so there is no live check to call. TikTok
//!                 favourites are stored and reachable from the offline roster;
//!                 they are never reported live.
//!
//! Channels already covered by `provider_follows` are skipped here, so a channel
//! that is both followed and favourited costs one platform call, not two.
//!
//! Results reach the frontend the same two ways the provider poller uses: a
//! `favorites-live-update` event carrying the whole snapshot, and, on an
//! offline -> live edge, `streamer-went-live` — tagged `source: "favorite"` so
//! the frontend can gate it on its own setting and tell the two watchers apart.

use crate::models::provider_stream::ProviderStream;
use crate::models::settings::AppState;
use crate::services::live_notification_service::LiveNotification;
use crate::services::providers::key::{make_key, parse_key, same_channel};
use crate::services::providers::registry;
use crate::services::twitch_service::TwitchService;
use log::debug;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use std::sync::atomic::Ordering;
use tokio::time::{interval, Duration, Instant};

/// Base tick. Each source has its own cadence on top of this.
const TICK: Duration = Duration::from_secs(30);

/// How many YouTube favourites one sweep will look at.
///
/// Deliberately well under `YouTubeSource::live_check`'s own 25: that function
/// fetches a watch page PER CHANNEL and warns in as many words that a burst of
/// them is what gets an IP challenged. The follows poller is already spending
/// that budget, and this is a second spender against the same address.
const YOUTUBE_PER_SWEEP: usize = 10;

/// Where the next capped YouTube sweep starts. Without this the cap examines the
/// same window forever and everything past it is invisible for the whole session.
static ROTATION: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn cadence_for(source: &str) -> Duration {
    match source {
        "twitch" => Duration::from_secs(60),
        "kick" => Duration::from_secs(60),
        "youtube" => Duration::from_secs(180),
        _ => Duration::from_secs(180),
    }
}

#[derive(Default)]
struct LiveState {
    /// Latest row per platform key, served to the frontend on demand.
    ///
    /// Only LIVE rows are ever stored, so this doubles as the live set for
    /// offline -> live edge detection. The provider poller keeps a separate
    /// `live_keys` and partitions it by a `"<provider>:"` prefix; that trick
    /// can't work here because Twitch rows are keyed by a BARE login (to dedupe
    /// against follow rows), so the source is read off the row instead.
    snapshot: HashMap<String, ProviderStream>,
}

static STATE: once_cell::sync::Lazy<Arc<RwLock<LiveState>>> =
    once_cell::sync::Lazy::new(|| Arc::new(RwLock::new(LiveState::default())));

/// The current who's-live snapshot across every favourite, for initial paint.
pub async fn snapshot() -> Vec<ProviderStream> {
    STATE.read().await.snapshot.values().cloned().collect()
}

/// What one sweep needs from settings, read under the lock and carried out of it.
///
/// Cloned and the lock DROPPED before any network call: `settings` is a
/// std::sync::Mutex, and holding one across an await is how a poller stalls
/// every other writer. `provider_live_service` reads its follow list the same
/// way for the same reason.
struct SweepPlan {
    twitch_ids: Vec<String>,
    /// provider -> channels to check.
    by_provider: HashMap<String, Vec<String>>,
    /// Row keys that a follow poller is ALSO watching. Swept here for the list,
    /// but never announced from here, or a go-live would toast twice.
    also_followed: HashSet<String>,
    /// Whether a go-live edge should be announced. Read here with the rest of
    /// the settings so the sweep never has to reach for the lock again.
    notify: bool,
}

fn plan(state: &AppState) -> Option<SweepPlan> {
    let settings = match state.settings.lock() {
        Ok(s) => s,
        Err(e) => {
            debug!("[FavoriteLive] settings lock poisoned: {}", e);
            return None;
        }
    };

    let mut twitch_ids = Vec::new();
    let mut by_provider: HashMap<String, Vec<String>> = HashMap::new();
    let mut also_followed: HashSet<String> = HashSet::new();

    for entry in &settings.favorite_streamers {
        // A recognised `provider:` prefix means a provider favourite; anything
        // else is a bare Twitch user id, which is how every favourite that
        // predates multi-platform support is stored.
        let parsed = parse_key(entry);
        if parsed.provider == "twitch" {
            twitch_ids.push(parsed.channel);
            continue;
        }

        // EVERY favourite is swept, including ones you also follow.
        //
        // This used to skip followed channels on the theory that the follow
        // poller already covered them. It does not, and the gap is not
        // theoretical: for a `native_follows` platform (YouTube while signed in)
        // that poller reads the SUBSCRIPTIONS FEED, which is ordered by recency
        // and admits in its own comment that a stream live for days sits below
        // today's uploads. So a favourite could be live, be missed by the feed,
        // and appear nowhere — while the same channel favourited on Kick showed
        // up instantly, because Kick has no native follow list and gets the very
        // live_check we would have run.
        //
        // Favourites are a short, explicitly chosen list. Polling them on their
        // own terms is what makes the feature mean the same thing on every
        // platform, and it is affordable precisely because the list is short.
        let followed = settings.provider_follows.iter().any(|f| {
            f.provider == parsed.provider && same_channel(&parsed.provider, &f.channel, &parsed.channel)
        });
        if followed {
            // Swept, but NOT announced: the follow poller owns the go-live toast
            // for a channel you follow, and both watchers firing would double it.
            // Keyed the way the resulting row will be, so the match is exact.
            also_followed.insert(make_key(&parsed.provider, &parsed.channel));
        }

        by_provider
            .entry(parsed.provider)
            .or_default()
            .push(parsed.channel);
    }

    Some(SweepPlan {
        twitch_ids,
        by_provider,
        also_followed,
        // Both gates, the same pair the follows poller answers to: the master
        // switch, and this watcher's own type toggle.
        notify: settings.live_notifications.enabled
            && settings.live_notifications.show_favorite_live_notifications,
    })
}

/// A Twitch row in the one shape the event carries, so the frontend doesn't have
/// to branch on platform to read the snapshot.
fn twitch_row(s: crate::models::stream::TwitchStream) -> ProviderStream {
    ProviderStream {
        provider: "twitch".to_string(),
        // Twitch keys stay BARE logins (no `twitch:` prefix), matching
        // `streamKey` on the frontend, so a favourite row dedupes against a
        // follow row for the same channel instead of rendering twice.
        key: s.user_login.to_lowercase(),
        watch_url: format!("https://twitch.tv/{}", s.user_login),
        id: s.id,
        user_id: s.user_id,
        user_login: s.user_login,
        user_name: s.user_name,
        title: s.title,
        viewer_count: s.viewer_count,
        game_id: s.game_id,
        game_name: s.game_name,
        category_thumbnail: None,
        thumbnail_url: s.thumbnail_url,
        started_at: s.started_at,
        profile_image_url: s.profile_image_url,
        is_live: true,
        tags: s.tags,
    }
}

/// Sweep everything immediately, outside the poll cadence.
///
/// Favouriting a channel that is live right now should put it in the sidebar
/// now, not up to a minute later. Same reasoning (and same `notify: false`) as
/// `provider_live_service::refresh_provider`: a manual refresh paints the list,
/// it does not fire a go-live toast for a stream that was already running.
pub async fn refresh_favorites(app: AppHandle, state: AppState) {
    let Some(plan) = plan(&state) else { return };
    for (source, rows, examined) in collect(&plan).await {
        apply(&app, &source, rows, false, &plan.also_followed, examined.as_ref()).await;
    }
}

/// Run the checks for every source that has work, returning
/// `(source, rows, examined)`. `examined` is `None` for a COMPLETE sweep and
/// `Some(keys)` when the sweep was capped, so `apply` knows which previously
/// live rows it must KEEP rather than read as having gone offline.
#[allow(clippy::type_complexity)]
async fn collect(
    plan: &SweepPlan,
) -> Vec<(String, Vec<ProviderStream>, Option<HashSet<String>>)> {
    let mut out = Vec::new();

    if !plan.twitch_ids.is_empty() {
        match TwitchService::get_streams_by_user_ids(&plan.twitch_ids).await {
            Ok(streams) => {
                // One line per sweep, deliberately at INFO. Diagnosing "I
                // favourited it and nothing happened" with only failure-path
                // logging means you cannot tell a sweep that found nobody from a
                // sweep that never ran.
                log::info!(
                    "[FavoriteLive] twitch: checked {} favourite(s), {} live",
                    plan.twitch_ids.len(),
                    streams.len()
                );
                // `None` = a COMPLETE sweep: Twitch batches 100 ids per call
                // with no cap, so every favourite was examined.
                out.push((
                    "twitch".to_string(),
                    streams.into_iter().map(twitch_row).collect(),
                    None,
                ));
            }
            // Signed out is the common case here, not an error worth shouting
            // about once a minute. The offline roster still lists these.
            Err(e) => debug!("[FavoriteLive] twitch check skipped: {}", e),
        }
    }

    let reg = registry().await;
    for (provider, channels) in &plan.by_provider {
        let Some(src) = reg.get_source(provider) else {
            // TikTok lands here: a provider with chat and playback but no
            // StreamSource, so there is nothing to ask.
            debug!(
                "[FavoriteLive] {} has {} favourite(s) but no live check exists; \
                 they are unchecked, NOT offline",
                provider,
                channels.len()
            );
            continue;
        };
        if !src.caps().live_check {
            continue;
        }
        // Capped sweeps ROTATE. `take(N)` read as reasonable and was in fact
        // permanent starvation: it examined the same first N every sweep, so
        // favourite N+1 was never checked once, ever - and an unchecked channel
        // reads as offline, which is a wrong answer rather than a slow one.
        let mut partial: Option<HashSet<String>> = None;
        let checked: Vec<String> = if provider == "youtube" && channels.len() > YOUTUBE_PER_SWEEP {
            // Say what was dropped. Reported as silence, "we looked at 10 of
            // your 30" and "nobody is live" are indistinguishable — the same
            // reasoning the YouTube source itself spells out at its own cap.
            log::warn!(
                "[FavoriteLive] checking {} of {} YouTube favourites this sweep; \
                 the rest are NOT being reported as offline, they are unchecked",
                YOUTUBE_PER_SWEEP,
                channels.len()
            );
            let start = ROTATION.fetch_add(YOUTUBE_PER_SWEEP, Ordering::Relaxed) % channels.len();
            let window: Vec<String> = channels
                .iter()
                .cycle()
                .skip(start)
                .take(YOUTUBE_PER_SWEEP)
                .cloned()
                .collect();
            partial = Some(window.iter().map(|c| make_key(provider, c)).collect());
            window
        } else {
            channels.clone()
        };

        match src.live_check(&checked).await {
            Ok(rows) => {
                log::info!(
                    "[FavoriteLive] {}: checked {} favourite(s), {} live",
                    provider,
                    checked.len(),
                    rows.iter().filter(|r| r.is_live).count()
                );
                out.push((provider.clone(), rows, partial));
            }
            Err(e) => debug!("[FavoriteLive] {} live_check failed: {}", provider, e),
        }
    }

    out
}

pub fn start(app: AppHandle, state: AppState) {
    // `tauri::async_runtime::spawn`, NOT `tokio::spawn`: the setup hook runs on
    // the main thread outside any runtime context, so a bare tokio spawn panics
    // with "there is no reactor running".
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(TICK);
        let mut last_run: HashMap<String, Instant> = HashMap::new();
        // The first sweep only populates: without this every favourite that
        // happens to be live at launch would fire a "went live" toast.
        let mut primed = false;

        loop {
            tick.tick().await;

            let Some(full) = plan(&state) else { continue };

            // Prune against the FULL favourites list, BEFORE narrowing to what
            // is due. Pruning against the narrowed plan would read "only Twitch
            // is due this tick" as "there are no Kick favourites" and drop every
            // Kick row from the snapshot, which the very next emit would publish
            // as the whole truth.
            prune_absent(&full).await;

            // Narrow the plan to the sources actually due, so a 60s Kick cadence
            // doesn't drag YouTube's 180s one along with it.
            let due = |source: &str| {
                last_run
                    .get(source)
                    .map(|t| t.elapsed() >= cadence_for(source))
                    .unwrap_or(true)
            };

            let plan = SweepPlan {
                twitch_ids: if due("twitch") {
                    full.twitch_ids
                } else {
                    Vec::new()
                },
                by_provider: full
                    .by_provider
                    .into_iter()
                    .filter(|(p, _)| due(p))
                    .collect(),
                also_followed: full.also_followed,
                notify: full.notify,
            };

            if plan.twitch_ids.is_empty() && plan.by_provider.is_empty() {
                continue;
            }

            if !plan.twitch_ids.is_empty() {
                last_run.insert("twitch".to_string(), Instant::now());
            }
            for provider in plan.by_provider.keys() {
                last_run.insert(provider.clone(), Instant::now());
            }

            for (source, rows, examined) in collect(&plan).await {
                apply(
                    &app,
                    &source,
                    rows,
                    primed && plan.notify,
                    &plan.also_followed,
                    examined.as_ref(),
                )
                .await;
            }

            primed = true;
        }
    });
}

/// Drop snapshot rows for channels that are no longer favourited.
///
/// `apply` clears a source's rows wholesale each sweep, so this only matters for
/// a source that has dropped to zero favourites and is therefore never swept
/// again — without it, its last known rows would linger for the session.
async fn prune_absent(plan: &SweepPlan) {
    let mut wanted: HashSet<String> = HashSet::new();
    for id in &plan.twitch_ids {
        wanted.insert(format!("twitch:{}", id));
    }
    for (provider, channels) in &plan.by_provider {
        for c in channels {
            wanted.insert(make_key(provider, c));
        }
    }
    let mut st = STATE.write().await;
    // Rows are keyed by PLATFORM key, favourites by favourite key, and for
    // YouTube those differ (a live row is keyed by video id, a favourite by UC
    // id). So match on either identity the row carries rather than comparing
    // the stored key against `wanted` directly.
    let stale: Vec<String> = st
        .snapshot
        .iter()
        .filter(|(_, row)| {
            let by_id = format!("{}:{}", row.provider, row.user_id);
            let by_login = make_key(&row.provider, &row.user_login);
            !wanted.contains(&by_id) && !wanted.contains(&by_login)
        })
        .map(|(k, _)| k.clone())
        .collect();
    for k in stale {
        st.snapshot.remove(&k);
    }
}

/// Fold one source's results into the shared state, emit the list update, and
/// fire go-live notifications for favourites that just came online.
async fn apply(
    app: &AppHandle,
    source: &str,
    rows: Vec<ProviderStream>,
    notify: bool,
    suppress_notify: &HashSet<String>,
    examined: Option<&HashSet<String>>,
) {
    let live: Vec<ProviderStream> = rows.into_iter().filter(|r| r.is_live).collect();
    let mut fresh_live = Vec::new();
    let rows_changed;
    let full: Vec<ProviderStream>;

    {
        let mut st = STATE.write().await;

        // This source's outgoing rows. Everything stored is live, so this is
        // both the comparison set and the "was it already live" set.
        let previously: HashMap<String, ProviderStream> = st
            .snapshot
            .iter()
            .filter(|(_, v)| v.provider == source)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Compare only within what this sweep could actually speak to, or a
        // rotating cap would report a change on every pass simply because the
        // window moved.
        let comparable: HashMap<&String, &ProviderStream> = previously
            .iter()
            .filter(|(k, _)| examined.map(|e| e.contains(*k)).unwrap_or(true))
            .map(|(k, v)| (k, v))
            .collect();
        rows_changed = comparable.len() != live.len()
            || live.iter().any(|r| comparable.get(&r.key).copied() != Some(r));
        drop(comparable);

        // Drop this source's previous rows so channels that went offline (or
        // were unfavourited) disappear, leaving other sources untouched.
        //
        // EXCEPT the ones this sweep never looked at. A capped sweep rotates
        // through the list, so most of it is unexamined on any given pass, and
        // treating unexamined as offline would make live favourites blink in and
        // out at the cap's cadence. Unchecked is not offline - the same
        // distinction the cap's own warning draws.
        st.snapshot.retain(|key, v| {
            v.provider != source || examined.map(|e| !e.contains(key)).unwrap_or(false)
        });
        for row in &live {
            st.snapshot.insert(row.key.clone(), row.clone());
            if !previously.contains_key(&row.key) {
                fresh_live.push(row.clone());
            }
        }

        // The event carries the WHOLE snapshot, not just this source's slice:
        // the frontend store replaces its map wholesale, and sending one source
        // at a time would make each sweep erase the others.
        full = st.snapshot.values().cloned().collect();
    }

    if rows_changed {
        let _ = app.emit(
            "favorites-live-update",
            serde_json::json!({ "source": source, "streams": full }),
        );
    }

    if !notify {
        return;
    }
    for row in fresh_live {
        // A channel you also FOLLOW is announced by the follow poller. Swept
        // here for the list, silent here for the toast.
        if suppress_notify.contains(&row.key) {
            continue;
        }
        let _ = app.emit(
            "streamer-went-live",
            LiveNotification {
                streamer_name: row.user_name.clone(),
                // Twitch rows carry a bare login and provider rows the composite
                // key, exactly as the two existing emitters do, so the click
                // handler routes to the right platform.
                streamer_login: row.key.clone(),
                streamer_avatar: row.profile_image_url.clone(),
                game_name: Some(row.game_name.clone()).filter(|g| !g.is_empty()),
                game_image: None,
                stream_title: Some(row.title.clone()).filter(|t| !t.is_empty()),
                stream_url: row.watch_url.clone(),
                is_test: false,
                source: Some("favorite".to_string()),
            },
        );
    }
}
