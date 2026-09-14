//! Android-only bridge for work that runs with no Tauri app around it.
//!
//! Notifications have to keep arriving after the app is closed, and at that
//! point there is no WebView to listen for an event and no `AppHandle` to emit
//! one. A Kotlin worker calls straight into this library instead, so everything
//! here must work with nothing but a JVM pointer: no app handle, no running
//! event loop, and no assumption that Tauri's activity ever started.
//!
//! Verified on device 2026-08-02: `System.loadLibrary("streamnook_lib")` and a
//! JNI call both work from a WorkManager worker in a process Android started for
//! the job alone, in a release (R8) build.
//!
//! The reason this lives in Rust rather than in the worker's own Kotlin is
//! `TwitchService::get_token()`, which loads, checks expiry, refreshes against
//! the compiled-in client secret, and persists. Handing Kotlin a raw token
//! instead would mean the background poll dies the first time one expires while
//! the app has not been opened.
#![cfg(target_os = "android")]

use jni::objects::{JObject, JString};
use jni::sys::{jboolean, jstring};
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::services::app_paths;
use crate::services::cache_service;
use crate::services::twitch_service::TwitchService;

/// Notified channels are remembered this long. Long enough that a channel which
/// drops off the followed-live list and returns is not announced twice, short
/// enough that the file cannot grow without bound.
const SHOWN_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// The push lane's server. Registration is authed by the user's own Twitch
/// token; the server validates it against Twitch and stores only the user id,
/// device token and followed channel ids.
const NOTIFY_SERVER: &str = "https://notify.streamnook.app";

/// Serializes every read-modify-write of `notify_state.json`. The poll lane
/// (WorkManager -> `pollOnce`) and the push lane (`claimShow`) run in the same
/// process on different threads, so a process-local lock is sufficient — and
/// without it a claim landing between the poll's read and write is silently
/// dropped, which resurfaces as a duplicate notification far from the cause.
static STATE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Default, Serialize, Deserialize)]
struct ShownEntry {
    /// The stream's own start time. Keyed on this rather than on "is currently
    /// live" so a genuinely new broadcast is always announced while an ongoing
    /// one never is, no matter how the live list churns between polls.
    started_at: String,
    seen_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NotifyState {
    #[serde(default)]
    shown: HashMap<String, ShownEntry>,
}

/// Badge drops, from the same edge-cached feed the app polls as its socket
/// fallback (`src/services/badgeSocketService.ts`).
///
/// This mirrors `commands::badge_service::ingest_badge_drops` deliberately
/// rather than approximating it: the classifier decides between first sighting,
/// an opening earn window, a corrected writeup and a redundant re-push, and a
/// second implementation of those rules would drift.
///
/// The one thing skipped is `apply_drop`'s `badge-metadata-amended` emit, which
/// only tells open panels to refresh. There are none in a cold process, and both
/// of the storage calls beneath it need no app handle, so the gallery and the
/// enrichment cache still get written exactly as they would in the app.
async fn collect_badges() -> Vec<OutItem> {
    use crate::commands::badge_service::is_window_open;
    use crate::services::badge_polling_service::{self as feed, BadgeNotification, FeedAction};

    // The feed is an array of relay DROPS, each wrapping the badge under a
    // `badge` key alongside the drop id and timestamp — not an array of badges.
    // Deserializing straight into BadgeNotification fails, and because the
    // failure path here is "return nothing", it would have looked exactly like a
    // quiet feed rather than a bug. Shape confirmed against the frontend's own
    // consumer (`badgeSocketService.ts`, which maps `d.badge` before invoking).
    #[derive(Deserialize)]
    struct RelayDrop {
        badge: BadgeNotification,
    }

    let client = crate::services::http::client().clone();
    let Ok(resp) = client
        .get("https://modroom.streamnook.app/badges/latest.json")
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(wrapped) = resp.json::<Vec<RelayDrop>>().await else {
        return Vec::new();
    };
    let drops: Vec<BadgeNotification> = wrapped.into_iter().map(|d| d.badge).collect();
    if drops.is_empty() {
        return Vec::new();
    }

    // Same reasoning as the live seed: a fresh install must absorb the relay's
    // whole backlog silently rather than announce every badge ever dropped.
    let seeding = feed::is_empty().await;
    let mut out = Vec::new();

    for badge in &drops {
        let feed_id = badge.feed_id();
        let hash = badge.content_hash();
        let available = is_window_open(badge);
        let action = if seeding {
            FeedAction::SilentStore
        } else {
            feed::classify(&feed_id, &hash, available).await
        };
        if !action.stores() {
            continue;
        }

        let _ = crate::commands::badges::merge_pushed_badge_into_global_cache(badge).await;
        if let Some(enrichment) = &badge.enrichment {
            crate::commands::badge_metadata::store_enrichment_metadata(
                &badge.badge_set_id,
                &badge.badge_version,
                enrichment,
            )
            .await;
        }

        if matches!(action, FeedAction::NotifyNew | FeedAction::NotifyAvailable) {
            out.push(OutItem {
                channel_id: format!("badge:{feed_id}"),
                login: String::new(),
                title: format!("New badge: {}", badge.badge_name),
                body: badge.badge_description.clone().unwrap_or_default(),
                avatar: Some(badge.badge_image_url.clone()),
                channel: NOTIFY_CHANNEL_BADGES.to_string(),
            });
        }

        // After the store, so a failed store retries on the next poll.
        feed::record(&feed_id, &hash, action, available).await;
    }

    feed::persist().await;
    out
}

/// One notification for the worker to post.
///
/// No notification id: the worker derives it from `channel_id` with Java's own
/// `hashCode`, so re-posting for the same channel replaces rather than stacks.
/// Computing it here would need Java's hash semantics reimplemented in Rust for
/// no benefit.
#[derive(Debug, Serialize)]
struct OutItem {
    /// Identity for the notification, hashed by the worker into its id. Prefixed
    /// for badges so a badge and a channel can never collide on one id.
    channel_id: String,
    /// Twitch login to open on tap. Empty for anything that is not a channel.
    login: String,
    title: String,
    body: String,
    avatar: Option<String>,
    /// Which OS category to post under.
    channel: String,
}

/// OS notification channel ids. Mirrored in `src/mobile/notifications.ts` and in
/// `NotifyChannels.kt`; all three must agree or a notification lands in the
/// wrong category (or a default one the user cannot configure).
const NOTIFY_CHANNEL_LIVE: &str = "live-channels";
const NOTIFY_CHANNEL_BADGES: &str = "badges";

/// Work out which directory the app actually stores things in.
///
/// The worker passes `Context.getDataDir()`, which is what Tauri's
/// `app_data_dir()` resolves to today (`tauri-2.11.5/src/path/android.rs:138`
/// calls `getDataDir`). That is worth not taking on faith: it is one line in a
/// dependency, `filesDir` is the far more common Android convention and sits
/// one level below it, and if the two ever disagree the failure is completely
/// silent — every store resolves to a directory that has never been written to,
/// so the poll reports "not signed in" forever.
///
/// So probe instead of trusting: every store lives under `<base>/StreamNook`
/// (`cache_service::get_app_data_dir`), which makes that directory a reliable
/// marker for which candidate is real.
fn resolve_base(candidate: &str) -> Option<PathBuf> {
    if candidate.is_empty() {
        return None;
    }
    let direct = PathBuf::from(candidate);
    if direct.join("StreamNook").is_dir() {
        return Some(direct);
    }
    let nested = direct.join("files");
    if nested.join("StreamNook").is_dir() {
        return Some(nested);
    }
    None
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn state_path() -> Option<PathBuf> {
    cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("notify_state.json"))
}

fn read_state() -> NotifyState {
    state_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Whether this poll has ever run before.
///
/// The first run has nothing recorded, so every channel that happens to be live
/// reads as newly live. Someone following a few dozen channels would get their
/// whole following list dumped into the shade at once, which is both useless and
/// the kind of thing that gets notifications turned off for good. The first run
/// therefore records the current state and announces nothing, exactly like the
/// in-app service's own `first_run` branch.
fn state_exists() -> bool {
    state_path().map(|p| p.is_file()).unwrap_or(false)
}

fn write_state(state: &NotifyState) {
    if let Some(path) = state_path() {
        if let Ok(json) = serde_json::to_string(state) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Settings are read as loose JSON rather than through the `Settings` struct on
/// purpose: this path runs with no app around it, and a background poll should
/// degrade to "do nothing" if a field it does not care about fails to parse,
/// not refuse to run.
fn read_settings() -> serde_json::Value {
    cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("settings.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Liveness probe, called from `app.streamnook.NotifyBridge.ping`.
///
/// Kept after the spike it was written for: it isolates "the library loaded and
/// JNI resolved" from "the poll found nothing", which are otherwise identical
/// from the outside.
///
/// `NotifyBridge` is a Kotlin `object`, so its members compile to instance
/// methods on the class and the second argument is the singleton, not a
/// `JClass`. A `companion object` with `@JvmStatic` would put the symbol on
/// `NotifyBridge$Companion` and this binding would silently never resolve.
#[no_mangle]
pub extern "system" fn Java_app_streamnook_NotifyBridge_ping<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
) -> jstring {
    match env.new_string("streamnook-jni-ok") {
        Ok(s) => s.into_raw(),
        // Returning null rather than panicking: unwinding across the JNI
        // boundary is undefined, and the caller already treats null as failure.
        Err(_) => std::ptr::null_mut(),
    }
}

/// Poll for followed channels that have gone live since the last run.
///
/// Returns a JSON array of notifications to post, or `[]` for every "nothing to
/// do" case: not signed in, notifications switched off, no network, or the
/// foreground path having checked in recently. Errors are `[]` too — a
/// background poll that cannot reach Twitch should be quiet, not noisy.
///
/// `base_dir` is passed in rather than resolved here because the two sides
/// disagree about what it is: Tauri's `app_data_dir()` on Android is
/// `Context.getDataDir()` (`/data/user/0/<pkg>`), while the obvious Kotlin
/// choice, `filesDir`, is the `files/` directory beneath it. The app records
/// the resolved path at startup and the worker passes it straight back, so the
/// two can never drift.
#[no_mangle]
pub extern "system" fn Java_app_streamnook_NotifyBridge_pollOnce<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    base_dir: JString<'local>,
) -> jstring {
    let empty = |env: &mut JNIEnv<'local>| -> jstring {
        match env.new_string("[]") {
            Ok(s) => s.into_raw(),
            Err(_) => std::ptr::null_mut(),
        }
    };

    let candidate: String = match env.get_string(&base_dir) {
        Ok(s) => s.into(),
        Err(_) => return empty(&mut env),
    };
    let Some(base) = resolve_base(&candidate) else {
        // No StreamNook directory under either candidate means the app has
        // never completed a first run, so there is nothing to poll for anyway.
        return empty(&mut env);
    };
    // The heartbeat file is retired (this worker no longer stands down for a
    // running app); clear any leftover from builds that wrote it.
    let _ = std::fs::remove_file(base.join("StreamNook").join("notify_ping"));
    // OnceLock, first write wins. Harmless whether or not the app already set
    // it, since both sides end up at the same directory.
    app_paths::set_base(base);

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return empty(&mut env),
    };

    let items = runtime.block_on(collect_all());
    let json = serde_json::to_string(&items).unwrap_or_else(|_| "[]".to_string());
    match env.new_string(json) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// The push lane's gate: may this broadcast be shown, and record that it was.
///
/// Called from `SNMessagingService` for every FCM live message. Evaluates the
/// same settings the poll lane does (master switch, live toggle, mute list),
/// then atomically checks-and-records `(channel_id, started_at)` in
/// `notify_state.json` under `STATE_LOCK`. Returning true exactly once per
/// broadcast across every lane is the whole dedupe story: a duplicate push, a
/// push/poll overlap, or a server-side dedupe miss all collapse here.
///
/// One deliberate asymmetry with the poll: when NO state file exists yet (fresh
/// install), this returns true but records NOTHING. The poll's silent first-run
/// seed is keyed on the file's absence, and creating it here with a single
/// entry would make that first poll read as post-seed and dump the whole
/// followed-live list into the shade. A push is safe to show unseeded — it only
/// exists because a broadcast started just now — and the seed run records the
/// channel as live without announcing, so no duplicate follows.
#[no_mangle]
pub extern "system" fn Java_app_streamnook_NotifyBridge_claimShow<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    base_dir: JString<'local>,
    channel_id: JString<'local>,
    started_at: JString<'local>,
    login: JString<'local>,
) -> jboolean {
    let get = |env: &mut JNIEnv<'local>, s: &JString<'local>| -> Option<String> {
        env.get_string(s).ok().map(|v| v.into())
    };
    let (Some(candidate), Some(channel_id), Some(started_at), Some(login)) = (
        get(&mut env, &base_dir),
        get(&mut env, &channel_id),
        get(&mut env, &started_at),
        get(&mut env, &login),
    ) else {
        return 0;
    };
    let Some(base) = resolve_base(&candidate) else {
        return 0;
    };
    app_paths::set_base(base);

    let settings = read_settings();
    if !notif_flag(&settings, "enabled") || !notif_flag(&settings, "show_live_notifications") {
        return 0;
    }
    let muted = settings
        .get("live_notifications")
        .and_then(|n| n.get("muted_live_channels"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .any(|m| m.eq_ignore_ascii_case(&login))
        })
        .unwrap_or(false);
    if muted {
        return 0;
    }

    let _guard = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if !state_exists() {
        // See the doc comment: show, but leave seeding to the poll.
        return 1;
    }
    let now = now_secs();
    let mut state = read_state();
    if let Some(prev) = state.shown.get_mut(&channel_id) {
        if prev.started_at == started_at {
            prev.seen_at = now;
            write_state(&state);
            return 0;
        }
    }
    state.shown.insert(
        channel_id,
        ShownEntry {
            started_at,
            seen_at: now,
        },
    );
    write_state(&state);
    1
}

/// Register this device with the push server: current FCM token plus the ids
/// of every followed channel. Called from the shell after auth and whenever the
/// push toggle turns on; re-running is cheap and idempotent (the server
/// replaces the follow set wholesale).
///
/// "no-token" is a normal outcome, not an error: the token file is written by
/// Kotlin once Play services answers, which can trail the first boot. The next
/// launch registers; the poll lane delivers meanwhile.
#[tauri::command]
pub async fn push_register() -> Result<String, String> {
    let token_path = cache_service::get_app_data_dir()
        .map_err(|e| e.to_string())?
        .join("fcm_token");
    let Some(fcm_token) = std::fs::read_to_string(&token_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Ok("no-token".to_string());
    };

    let user_token = TwitchService::get_token().await.map_err(|e| e.to_string())?;

    // Every followed channel id, paged. Reuses the shared helper rather than
    // reimplementing the endpoint; the hard stop mirrors fetch_followed_live.
    let mut follows: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..20 {
        let (page, next) = TwitchService::get_all_followed_channels(100, cursor)
            .await
            .map_err(|e| e.to_string())?;
        follows.extend(page.into_iter().map(|c| c.user_id));
        cursor = next;
        if cursor.is_none() {
            break;
        }
    }

    let client = crate::services::http::client().clone();
    let resp = client
        .post(format!("{NOTIFY_SERVER}/register"))
        .header(
            reqwest::header::AUTHORIZATION,
            format!("OAuth {user_token}"),
        )
        .json(&serde_json::json!({ "fcm_token": fcm_token, "follows": follows }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("register failed: {status}"));
    }
    Ok(format!("registered:{}", follows.len()))
}

/// Drop this device from the push server (push toggle turned off). The poll
/// lane keeps delivering; this only stops the instant pushes.
#[tauri::command]
pub async fn push_unregister() -> Result<(), String> {
    let token_path = cache_service::get_app_data_dir()
        .map_err(|e| e.to_string())?
        .join("fcm_token");
    let Some(fcm_token) = std::fs::read_to_string(&token_path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    let user_token = TwitchService::get_token().await.map_err(|e| e.to_string())?;
    let client = crate::services::http::client().clone();
    client
        .post(format!("{NOTIFY_SERVER}/unregister"))
        .header(
            reqwest::header::AUTHORIZATION,
            format!("OAuth {user_token}"),
        )
        .json(&serde_json::json!({ "fcm_token": fcm_token }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Followed channels that are live right now, fully paged.
///
/// Deliberately NOT `TwitchService::get_followed_streams`: that one takes an
/// `AppState` (which it never reads, but there is no app here to build one
/// from), and it is shared with desktop, where changing its paging behaviour
/// would be a desktop change. Doing the request here keeps desktop untouched and
/// lets the background poll page properly.
///
/// Paging matters at the edges only: `first` defaults to 100 on this endpoint,
/// which is also its maximum, so a single page already covers almost everyone.
/// The cursor loop is for accounts with more than 100 followed channels live at
/// once, which the unpaged version silently truncates.
///
/// `get_token` is reused deliberately: it is what refreshes an expired token
/// against the compiled-in secret and re-persists it, and it is the entire
/// reason this poll lives in Rust rather than in the worker's Kotlin.
async fn fetch_followed_live() -> anyhow::Result<Vec<crate::models::stream::TwitchStream>> {
    const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
    let token = TwitchService::get_token().await?;
    let me = TwitchService::get_user_info().await?;
    let client = crate::services::http::client().clone();

    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    // A hard stop rather than `loop`: a malformed cursor that keeps returning
    // itself would otherwise spin a background job against Twitch forever.
    for _ in 0..10 {
        let mut url = format!(
            "https://api.twitch.tv/helix/streams/followed?user_id={}&first=100",
            me.id
        );
        if let Some(c) = &cursor {
            url.push_str(&format!("&after={}", c));
        }
        let resp = client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        match resp.get("data").and_then(|d| d.as_array()) {
            Some(arr) if !arr.is_empty() => {
                for v in arr {
                    if let Ok(s) =
                        serde_json::from_value::<crate::models::stream::TwitchStream>(v.clone())
                    {
                        out.push(s);
                    }
                }
            }
            _ => break,
        }

        cursor = resp
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
}

/// Avatars for the handful of channels actually being announced.
///
/// The followed-streams response carries a stream thumbnail, not a profile
/// image, so this is a second request — but only ever for the channels in this
/// batch, which is normally none or one, rather than for the whole live list.
async fn fetch_avatars(ids: &[String]) -> HashMap<String, String> {
    const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
    let mut map = HashMap::new();
    if ids.is_empty() {
        return map;
    }
    let Ok(token) = TwitchService::get_token().await else {
        return map;
    };
    let client = crate::services::http::client().clone();
    // The users endpoint caps at 100 ids per call.
    for chunk in ids.chunks(100) {
        let query = chunk
            .iter()
            .map(|id| format!("id={}", id))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("https://api.twitch.tv/helix/users?{}", query);
        let Ok(resp) = client
            .get(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await
        else {
            continue;
        };
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            continue;
        };
        if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
            for user in arr {
                if let (Some(id), Some(img)) = (
                    user.get("id").and_then(|v| v.as_str()),
                    user.get("profile_image_url").and_then(|v| v.as_str()),
                ) {
                    if !img.is_empty() {
                        map.insert(id.to_string(), img.to_string());
                    }
                }
            }
        }
    }
    map
}

/// Read a `live_notifications` boolean, defaulting to ON when absent.
///
/// Matching the serde defaults on the Rust struct matters here: a settings file
/// written before these fields existed must not read as "everything disabled".
fn notif_flag(settings: &serde_json::Value, name: &str) -> bool {
    settings
        .get("live_notifications")
        .and_then(|n| n.get(name))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Everything the background poll should announce this run.
///
/// The shared gates live here so the per-category collectors only decide their
/// own business, and so the heartbeat is checked exactly once no matter how many
/// categories get added later.
async fn collect_all() -> Vec<OutItem> {
    let settings = read_settings();
    // The single master switch. This worker is the only delivery lane whether
    // the app is open or not, so there is deliberately no "app is awake, stand
    // down" check anymore: the started_at dedupe below is what prevents a
    // double, not lane arbitration.
    if !notif_flag(&settings, "enabled") {
        return Vec::new();
    }

    let mut out = Vec::new();
    if notif_flag(&settings, "show_live_notifications") {
        out.extend(collect_live(&settings).await);
    }
    if notif_flag(&settings, "show_badge_notifications") {
        out.extend(collect_badges().await);
    }
    out
}

async fn collect_live(settings: &serde_json::Value) -> Vec<OutItem> {
    let notif = settings.get("live_notifications");

    let muted: Vec<String> = notif
        .and_then(|n| n.get("muted_live_channels"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    // Network happens BEFORE the state lock, so the lock never spans an await
    // and only covers the read-modify-write it exists for.
    let streams = match fetch_followed_live().await {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let now = now_secs();
    let mut out = Vec::new();
    {
        let _guard = STATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // Nothing recorded yet means this is the first poll since install (or
        // since the app's data was cleared). Seed silently: announcing here
        // would empty the whole following list into the shade at once.
        let seeding = !state_exists();
        let mut state = read_state();

        for stream in &streams {
            if muted.contains(&stream.user_login.to_lowercase()) {
                continue;
            }
            // Already announced THIS broadcast. A different `started_at` for
            // the same channel is a genuinely new stream and does notify.
            //
            // `seen_at` is bumped rather than left alone, which matters for
            // permanent streams: a channel live continuously for longer than
            // the TTL would otherwise be pruned as stale and announced again
            // while it was still broadcasting. Refreshing on every sighting
            // means an entry only ages out once the stream has actually been
            // gone that long.
            if let Some(prev) = state.shown.get_mut(&stream.user_id) {
                if prev.started_at == stream.started_at {
                    prev.seen_at = now;
                    continue;
                }
            }
            state.shown.insert(
                stream.user_id.clone(),
                ShownEntry {
                    started_at: stream.started_at.clone(),
                    seen_at: now,
                },
            );
            // Recorded above either way; the first poll records without
            // announcing.
            if seeding {
                continue;
            }
            let body = if !stream.title.is_empty() {
                stream.title.clone()
            } else if !stream.game_name.is_empty() {
                format!("Playing {}", stream.game_name)
            } else {
                String::new()
            };
            out.push(OutItem {
                channel_id: stream.user_id.clone(),
                login: stream.user_login.clone(),
                title: format!("{} is live", stream.user_name),
                body,
                avatar: None,
                channel: NOTIFY_CHANNEL_LIVE.to_string(),
            });
        }

        // Age out rather than dropping everything that is no longer live.
        // Pruning to the current live set looks tidier and is wrong: a channel
        // that falls off the list and comes back with the same `started_at`
        // would be announced a second time for a stream that never stopped.
        state
            .shown
            .retain(|_, e| now - e.seen_at < SHOWN_TTL_SECS);
        write_state(&state);
    }

    // State is written BEFORE the avatar lookup, so a failure there costs a
    // picture rather than replaying every notification on the next run.
    if !out.is_empty() {
        let ids: Vec<String> = out.iter().map(|i| i.channel_id.clone()).collect();
        let avatars = fetch_avatars(&ids).await;
        for item in &mut out {
            item.avatar = avatars.get(&item.channel_id).cloned();
        }
    }

    out
}
