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
use crate::services::live_poll::poll::{notif_flag, read_settings};
use crate::services::twitch_service::TwitchService;

// The live check itself, its `notify_state.json` record and the lock around
// it are shared with the iPhone in `services::live_poll`; this file keeps what
// only Android does with them (JNI, badges, avatars, notification channels).

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
            crate::commands::badge_service::heal_enrichment(badge).await;
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
    // The badge collector stores each drop's earn window in the universal
    // cache, whose manifest only reaches disk on a 30 s debounce. This process
    // is often gone well before that, while the feed state recording the drop
    // as seen is written at once, so the app would skip the drop as unchanged
    // and never learn its window: missing badges that never read Available.
    let _ = crate::services::universal_cache_service::flush_manifest_now();
    drop(runtime);
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
/// `notify_state.json` under the shared lock (`live_poll::poll::claim_push`;
/// a push's start time with milliseconds matches the poll's whole-second one).
/// Returning true exactly once per
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
    crate::services::live_poll::poll::claim_push(&channel_id, &started_at, &login) as jboolean
}

/// The push lane's gate for a mention the mention server pushed: may it show.
///
/// Called from `SNMessagingService` for every FCM `mention` message. The same
/// rules as the app's own mention notices (`mention_watch::claim_push`): the
/// master switch and Mentions and replies on, the chat not muted, streamer
/// mode off, and, when the app is running in this process, not a message it
/// already caught or a chat on screen. A running app also pulls the entry into
/// the inbox; a process Android started for the push alone leaves that to the
/// next launch.
#[no_mangle]
pub extern "system" fn Java_app_streamnook_NotifyBridge_claimMention<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    base_dir: JString<'local>,
    id: JString<'local>,
    room: JString<'local>,
) -> jboolean {
    let get = |env: &mut JNIEnv<'local>, s: &JString<'local>| -> Option<String> {
        env.get_string(s).ok().map(|v| v.into())
    };
    let (Some(candidate), Some(id), Some(room)) = (get(&mut env, &base_dir), get(&mut env, &id), get(&mut env, &room))
    else {
        return 0;
    };
    let Some(base) = resolve_base(&candidate) else {
        return 0;
    };
    app_paths::set_base(base);
    let show = crate::services::mention_watch::claim_push(&read_settings(), &id, &room);
    crate::services::mention_watch::client::on_push_arrived();
    show as jboolean
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
    // The shared check (`services::live_poll`): followed and live, not muted,
    // not announced before, the first run seeding silently.
    let mut out: Vec<OutItem> = crate::services::live_poll::poll::collect(settings, &[], None)
        .await
        .into_iter()
        .map(|alert| OutItem {
            channel_id: alert.channel_id,
            login: alert.login,
            title: alert.title,
            body: alert.body,
            avatar: None,
            channel: NOTIFY_CHANNEL_LIVE.to_string(),
        })
        .collect();

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

// ---------------------------------------------------------------------------
// Mention notifications while the app runs
// ---------------------------------------------------------------------------

/// The Kotlin side that turns a mention into a notification
/// (`app.streamnook.MentionNotifyPlugin`).
struct MentionNotifier<R: tauri::Runtime>(tauri::plugin::PluginHandle<R>);

/// Posts every mention `services::mention_inbox` decided to announce (its
/// `mention-notify` event: the setting, streamer mode, the per-chat cooldown,
/// replay age and an off-screen chat are already checked there).
///
/// Listened for here rather than in the page because wry pauses the WebView
/// whenever the app leaves the screen, which is exactly when a mention needs
/// to reach the shade. The Rust event bus still runs then, and so does the
/// activity's main looper that the plugin call lands on.
pub fn mention_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri::{Listener, Manager};
    tauri::plugin::Builder::new("mention-notify")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.streamnook", "MentionNotifyPlugin")?;
            app.manage(MentionNotifier(handle));
            let app_handle = app.clone();
            app.listen(crate::services::mention_inbox::NOTIFY_EVENT, move |event| {
                let Ok(entry) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
                    return;
                };
                let app = app_handle.clone();
                // `run_mobile_plugin` blocks until Kotlin answers; never on
                // the chat lane that emitted the event.
                tauri::async_runtime::spawn_blocking(move || {
                    let state = app.state::<MentionNotifier<R>>();
                    if let Err(e) = state.0.run_mobile_plugin::<serde_json::Value>("post", entry) {
                        log::warn!("[MentionNotify] post failed: {e}");
                    }
                });
            });
            // Whispers `whisper_inbox` decided to announce (the setting,
            // streamer mode, not the conversation on screen): the same plugin
            // posts them as a conversation with a reply box
            // (`WhisperNotifier`, answered by `replyWhisper` below).
            let whisper_app = app.clone();
            app.listen(crate::services::whisper_inbox::NOTIFY_EVENT, move |event| {
                let Ok(update) = serde_json::from_str::<serde_json::Value>(event.payload()) else {
                    return;
                };
                let app = whisper_app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let state = app.state::<MentionNotifier<R>>();
                    match state.0.run_mobile_plugin::<serde_json::Value>("postWhisper", update) {
                        Ok(_) => log::info!("[WhisperNotify] posted"),
                        Err(e) => log::warn!("[WhisperNotify] post failed: {e}"),
                    }
                });
            });
            Ok(())
        })
        .build()
}

/// A reply typed into a whisper notification (`WhisperReplyReceiver`).
/// Returns "" when Twitch took it, else the reason it was not sent, in the
/// words every platform shows.
///
/// With the app running this is exactly the desktop's notification reply:
/// `whisper_inbox::reply_from_notification`, which sends through the same
/// `send` as `whisper_send`, files the reply in the conversation and tells the
/// page. Android may have ended the process since the notification was posted;
/// a cold process has no app to file into, so it only sends (the reply is on
/// Twitch, and the conversation shows it once the thread is pulled again).
#[no_mangle]
pub extern "system" fn Java_app_streamnook_NotifyBridge_replyWhisper<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    base_dir: JString<'local>,
    user_id: JString<'local>,
    login: JString<'local>,
    name: JString<'local>,
    avatar: JString<'local>,
    text: JString<'local>,
) -> jstring {
    let get = |env: &mut JNIEnv<'local>, s: &JString<'local>| -> String {
        env.get_string(s).map(|v| v.into()).unwrap_or_default()
    };
    let (candidate, user_id, login, name, avatar, text) = (
        get(&mut env, &base_dir),
        get(&mut env, &user_id),
        get(&mut env, &login),
        get(&mut env, &name),
        get(&mut env, &avatar),
        get(&mut env, &text),
    );
    let answer = reply_whisper(&candidate, &user_id, &login, &name, avatar, text);
    match env.new_string(answer) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn reply_whisper(candidate: &str, user_id: &str, login: &str, name: &str, avatar: String, text: String) -> String {
    let text = text.trim().to_string();
    if text.is_empty() || user_id.is_empty() {
        return String::new();
    }
    let avatar = (!avatar.is_empty()).then_some(avatar);
    if let Some(app) = crate::services::providers::app_handle() {
        log::info!("[WhisperNotify] notification reply, sending through the inbox");
        let result = tauri::async_runtime::block_on(crate::services::whisper_inbox::reply_from_notification(
            &app, user_id, login, name, avatar, text,
        ));
        return result.err().unwrap_or_default();
    }
    log::info!("[WhisperNotify] notification reply in a cold process, sending only");
    if let Some(base) = resolve_base(candidate) {
        app_paths::set_base(base);
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        return "Failed to send message. Please try again.".into();
    };
    runtime
        .block_on(TwitchService::send_whisper(user_id, &text))
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default()
}
