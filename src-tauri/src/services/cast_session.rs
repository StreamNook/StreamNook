//! Casting the watched stream to a Google Cast device, as a session Rust owns.
//!
//! The TV plays the stream itself: it fetches Twitch's and Kick's playlists and
//! segments straight from their CDNs (both answer any origin with CORS `*` on
//! media playlists and segments), so nothing flows through the phone. The phone
//! is the remote. This module decides WHAT the TV plays and when it changes;
//! the Kotlin side (`CastRemote.kt`) owns the device session, because only
//! Google's Cast SDK can hold one, and reports what happens to it through
//! `CastBridge.onEvent`.
//!
//! Why the lookup is done here and not taken from `start_stream`: the phone's
//! own player is handed a localhost relay URL, which a TV cannot reach. The TV
//! needs the upstream media playlist, picked for the TV (H.264, the TV's own
//! ceiling) rather than for this phone's decoder and screen.
//!
//! While a session is up the phone's own relay is stopped (its low-latency
//! reader downloads the stream continuously on its own) and `start_stream`
//! returns a cast placeholder instead of starting it again; the page shows the
//! casting card in place of the video.
//!
//! Events:
//! - `cast://state` the whole [`CastState`], on every change.
#![cfg(target_os = "android")]

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::cast_pick::{pick_rendition, Rendition};
use crate::services::twitch_resolver as tr;
use crate::services::watch_session::WatchTarget;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

pub const STATE_EVENT: &str = "cast://state";

/// Whether this build offers Cast, screen mirroring and TV screen mode at all.
/// Off until they have been tested on a device: the page hides the player's TV
/// button (it reads [`CastState::offered`]), a Cast session the SDK resumes is
/// ignored, and a mirrored TV keeps showing the plain phone screen. Setting it
/// to true turns all three back on; nothing else gates them.
pub const OFFERED: bool = false;

/// The stream id the Kick adapter is asked to resolve under. Its resolve is
/// pure, but the id keeps a cast lookup from ever being mistaken for the
/// phone's solo stream in a log.
const CAST_STREAM_ID: &str = "cast";

/// A relayed Twitch stream is spliced with the viewer's own above-1080p tiers,
/// and those are guarded only by the ad filter, which runs on the phone. The TV
/// plays them unfiltered, so a relayed stream stays on the relay's own tiers.
const RELAYED_HEIGHT_CAP: u32 = 1080;

/// How many times a stream the TV refused is looked up again (an expired
/// playlist token is the usual cause) before the error is shown, and over what
/// window those attempts are counted.
const RELOAD_ATTEMPTS: u32 = 3;
const RELOAD_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// No device session.
    #[default]
    Idle,
    /// The user picked a device and the SDK is connecting to it.
    Connecting,
    /// A device session is up. `target` says what it plays, if anything yet.
    Casting,
}

/// What the TV is playing, as the page shows it and as the receiver carries it
/// back on a resumed session (it rides the load's `customData`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CastTarget {
    pub provider: String,
    pub login: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
}

impl CastTarget {
    fn same_stream(&self, other: &CastTarget) -> bool {
        self.provider == other.provider && self.login.eq_ignore_ascii_case(&other.login)
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CastState {
    /// Whether this build offers Cast and mirroring at all ([`OFFERED`]).
    pub offered: bool,
    /// Whether this phone can cast at all (Google Play services present).
    pub available: bool,
    pub phase: Phase,
    /// The device's friendly name ("Living room TV").
    pub device: Option<String>,
    /// A speaker: it gets the audio-only rendition, never video.
    pub audio_only: bool,
    pub target: Option<CastTarget>,
    /// The receiver's player: "loading" | "playing" | "paused" | "buffering" | "idle".
    pub player: String,
    /// Why the last thing failed, worded for the viewer.
    pub error: Option<String>,
}

/// One event from `CastRemote.kt`, as JSON over JNI.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NativeEvent {
    /// Whether the Cast SDK came up (it cannot without Play services).
    Available { value: bool },
    /// "starting" | "started" | "resumed" | "ending" | "ended" | "failed" | "suspended".
    Session {
        state: String,
        #[serde(default)]
        device: String,
        #[serde(default = "yes")]
        video_out: bool,
    },
    /// The receiver's player state. `custom` is the target this app loaded,
    /// read back from the receiver (how a resumed session learns what plays).
    Media {
        state: String,
        #[serde(default)]
        idle_reason: String,
        #[serde(default)]
        custom: Option<CastTarget>,
    },
    /// The SDK refused or failed a load before the receiver reported anything.
    LoadFailed {
        #[serde(default)]
        code: i32,
    },
    /// `TvScreen.kt`: a mirrored display that takes a Presentation appeared,
    /// changed, or went away (`present: false`).
    Display {
        #[serde(default)]
        present: bool,
        #[serde(default)]
        name: String,
        #[serde(default)]
        width: u32,
        #[serde(default)]
        height: u32,
    },
    /// `TvScreen.kt`: the TV player's state.
    TvPlayer {
        state: String,
        #[serde(default)]
        error: Option<String>,
    },
}

fn yes() -> bool {
    true
}

struct Inner {
    state: CastState,
    /// A resumed session does not say what the TV plays until the first media
    /// status arrives; until then a status of "idle" means nothing was loaded.
    awaiting_resumed_media: bool,
    reloads: Vec<Instant>,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static INNER: Mutex<Option<Inner>> = Mutex::new(None);
/// Kotlin's events, handled one at a time and in the order they were sent: a
/// resumed session reports "resumed" and then its media status, and the second
/// means something different without the first.
static QUEUE: OnceLock<tokio::sync::mpsc::UnboundedSender<NativeEvent>> = OnceLock::new();
/// Events that arrived from Kotlin before Tauri's setup handed us the app (a
/// session the SDK resumes while the activity is still starting). Also the lock
/// that orders a late event against `init` draining this list.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Bumped by every load; a lookup that finishes after a newer one started is dropped.
static LOAD_SEQ: AtomicU64 = AtomicU64::new(0);

fn with_inner<R>(f: impl FnOnce(&mut Inner) -> R) -> R {
    let mut guard = INNER.lock().unwrap_or_else(|e| e.into_inner());
    let inner = guard.get_or_insert_with(|| Inner {
        state: CastState { player: "idle".into(), ..Default::default() },
        awaiting_resumed_media: false,
        reloads: Vec::new(),
    });
    f(inner)
}

fn emit(app: &AppHandle) {
    let _ = app.emit(STATE_EVENT, state());
}

/// Called once from setup. Starts the event worker and replays anything Kotlin
/// reported before it.
pub fn init(app: AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<NativeEvent>();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            handle(&app, event);
        }
    });
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    for json in pending.drain(..) {
        if let Some(event) = parse(&json) {
            let _ = tx.send(event);
        }
    }
    let _ = QUEUE.set(tx);
}

pub fn state() -> CastState {
    CastState { offered: OFFERED, ..with_inner(|i| i.state.clone()) }
}

/// The app handle, once setup has run. TV screen mode borrows it, since it is
/// told about streams from `start_stream`, which has none to give.
pub(crate) fn app() -> Option<AppHandle> {
    APP.get().cloned()
}

/// Platforms whose streams a Cast device fetches on its own: their CDNs answer
/// any origin on media playlists and segments. YouTube's does not, and TikTok
/// plays through the phone's own relay.
fn castable(provider: &str) -> bool {
    matches!(provider, "twitch" | "kick")
}

/// Whether `start_stream` should leave this stream to the TV: a session is
/// connected (not merely connecting, so a connection that fails leaves the
/// phone's player untouched) and the stream is one the TV can fetch.
pub fn takes_stream(provider: &str) -> bool {
    castable(provider) && with_inner(|i| i.state.phase == Phase::Casting)
}

fn parse(json: &str) -> Option<NativeEvent> {
    match serde_json::from_str(json) {
        Ok(e) => Some(e),
        Err(e) => {
            log::warn!("[Cast] unreadable event {json}: {e}");
            None
        }
    }
}

/// Entry point for `CastBridge.onEvent`, on the main thread: parse and queue.
pub fn on_native_event(json: String) {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    match QUEUE.get() {
        Some(tx) => {
            drop(pending);
            if let Some(event) = parse(&json) {
                let _ = tx.send(event);
            }
        }
        None => pending.push(json),
    }
}

/// One event, in order. Never awaits: a stream lookup can take as long as the
/// stream timeout, so loads run on their own tasks (their sequence number drops
/// any that a newer one overtook) and the queue keeps moving.
fn handle(app: &AppHandle, event: NativeEvent) {
    match event {
        // Not offered: a session the SDK kept from a test install never takes
        // the phone's stream or shows the casting card.
        NativeEvent::Session { .. } | NativeEvent::Media { .. } | NativeEvent::LoadFailed { .. } if !OFFERED => {}
        NativeEvent::Available { value } => {
            with_inner(|i| i.state.available = value);
            emit(app);
        }
        NativeEvent::Session { state, device, video_out } => on_session(app, &state, device, video_out),
        NativeEvent::Media { state, idle_reason, custom } => on_media(app, state, idle_reason, custom),
        NativeEvent::LoadFailed { code } => {
            log::warn!("[Cast] load failed (status {code})");
            reload_or_fail(app, "The TV couldn't open this stream.");
        }
        NativeEvent::Display { present, name, width, height } => {
            let display = present.then(|| crate::services::tv_screen::TvDisplay { name, width, height });
            crate::services::tv_screen::on_display(app, display);
        }
        NativeEvent::TvPlayer { state, error } => crate::services::tv_screen::on_player(app, state, error),
    }
}

fn spawn_load(app: &AppHandle, target: CastTarget) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { load(app, target).await });
}

fn on_session(app: &AppHandle, state: &str, device: String, video_out: bool) {
    log::info!("[Cast] session {state} ({device}, video_out={video_out})");
    match state {
        "starting" => {
            with_inner(|i| {
                i.state.phase = Phase::Connecting;
                i.state.device = Some(device).filter(|d| !d.is_empty());
                i.state.error = None;
            });
            emit(app);
        }
        "started" | "resumed" => {
            let resumed = state == "resumed";
            with_inner(|i| {
                i.state.phase = Phase::Casting;
                if !device.is_empty() {
                    i.state.device = Some(device);
                }
                i.state.audio_only = !video_out;
                i.state.error = None;
                i.reloads.clear();
                i.awaiting_resumed_media = resumed;
            });
            emit(app);
            // A Cast device takes the stream over from TV screen mode.
            crate::services::tv_screen::apply(app);
            // Only a stream the TV can take over is handed to it. Anything else
            // (YouTube, say) keeps playing on the phone, and the TV picks up the
            // next Twitch or Kick stream the viewer opens.
            let Some(target) = watched_target().filter(|t| castable(&t.provider)) else { return };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // The phone's own relay keeps downloading by itself (the LL
                // reader); the TV has the stream now.
                if let Err(e) = crate::commands::streaming::stop_stream().await {
                    log::debug!("[Cast] stopping the phone's relay: {e}");
                }
                // A resumed session already plays something; its first media
                // status says what (on_media).
                if !resumed {
                    load(app, target).await;
                }
            });
        }
        // Suspended is the SDK losing the connection for a moment; it resumes
        // or ends on its own.
        "suspended" | "ending" => {}
        "failed" => {
            let name = device_label(&device);
            reset(app, Some(format!("Couldn't connect to {name}.")));
        }
        _ => reset(app, None),
    }
}

fn on_media(app: &AppHandle, state: String, idle_reason: String, custom: Option<CastTarget>) {
    let resumed_first = with_inner(|i| std::mem::take(&mut i.awaiting_resumed_media));
    if resumed_first {
        if state != "idle" {
            if let Some(target) = custom {
                with_inner(|i| i.state.target = Some(target));
            }
        } else if let Some(target) = watched_target().filter(|t| castable(&t.provider)) {
            // A resumed session with nothing on the receiver: cast what the
            // phone was showing.
            with_inner(|i| i.state.player = state.clone());
            emit(app);
            spawn_load(app, target);
            return;
        }
    }
    with_inner(|i| i.state.player = state.clone());
    emit(app);

    if state == "idle" && with_inner(|i| i.state.phase == Phase::Casting && i.state.target.is_some()) {
        match idle_reason.as_str() {
            "error" => reload_or_fail(app, "The TV couldn't play this stream."),
            // A live stream that finished has ended. The watch session confirms
            // it and switches, which reaches the TV through `on_watch_target`.
            "finished" => {
                with_inner(|i| i.state.error = Some("The stream ended.".into()));
                emit(app);
            }
            _ => {}
        }
    }
}

/// Look the current target up again, at most [`RELOAD_ATTEMPTS`] times per
/// [`RELOAD_WINDOW`]; past that, show `message`.
fn reload_or_fail(app: &AppHandle, message: &str) {
    let retry = with_inner(|i| {
        let now = Instant::now();
        i.reloads.retain(|t| now.duration_since(*t) < RELOAD_WINDOW);
        if i.reloads.len() as u32 >= RELOAD_ATTEMPTS {
            return None;
        }
        i.reloads.push(now);
        i.state.target.clone()
    });
    match retry {
        Some(target) => spawn_load(app, target),
        None => {
            with_inner(|i| i.state.error = Some(message.to_string()));
            emit(app);
        }
    }
}

fn reset(app: &AppHandle, error: Option<String>) {
    LOAD_SEQ.fetch_add(1, Ordering::AcqRel);
    with_inner(|i| {
        let available = i.state.available;
        i.state = CastState { available, player: "idle".into(), error, ..Default::default() };
        i.awaiting_resumed_media = false;
        i.reloads.clear();
    });
    emit(app);
    crate::services::tv_screen::apply(app);
}

fn device_label(device: &str) -> String {
    if device.is_empty() {
        "the device".to_string()
    } else {
        device.to_string()
    }
}

fn watched_target() -> Option<CastTarget> {
    crate::services::watch_session::current_target().map(|t| target_of(&t))
}

fn target_of(t: &WatchTarget) -> CastTarget {
    CastTarget {
        provider: t.provider.clone(),
        login: t.login.to_lowercase(),
        name: if t.user_name.is_empty() { t.login.clone() } else { t.user_name.clone() },
        title: t.title.clone(),
    }
}

// ---- following the watch session ------------------------------------------------

/// The view started a stream. While casting, the TV follows it, if it is one
/// the TV can fetch; anything else plays on the phone and the TV keeps what it
/// has. The same stream again (the view catching up with a raid the TV already
/// followed) only fills in the name and title the raid did not carry.
pub fn on_watch_target(app: &AppHandle, target: &WatchTarget) {
    if !castable(&target.provider) {
        return;
    }
    let next = target_of(target);
    let follow = with_inner(|i| {
        if i.state.phase != Phase::Casting {
            return None;
        }
        match i.state.target.as_mut() {
            Some(t) if t.same_stream(&next) => {
                t.name = next.name.clone();
                t.title = next.title.clone();
                Some(false)
            }
            _ => Some(true),
        }
    });
    match follow {
        Some(true) => spawn_load(app, next),
        Some(false) => emit(app),
        None => {}
    }
}

/// The watched Twitch channel raided out. The TV follows straight away, so a
/// locked phone whose page is slow to act on the redirect still moves it; the
/// view's own start for the same channel then finds it already loaded.
pub fn on_raid(app: &AppHandle, to_login: &str, to_name: &str) {
    let casting_twitch = with_inner(|i| {
        i.state.phase == Phase::Casting
            && i.state.target.as_ref().is_some_and(|t| t.provider == "twitch")
    });
    if !casting_twitch || to_login.is_empty() {
        return;
    }
    let next = CastTarget {
        provider: "twitch".into(),
        login: to_login.to_lowercase(),
        name: if to_name.is_empty() { to_login.to_string() } else { to_name.to_string() },
        title: String::new(),
    };
    spawn_load(app, next);
}

// ---- loading a stream onto the receiver -----------------------------------------

async fn load(app: AppHandle, target: CastTarget) {
    let seq = LOAD_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    let audio_only = with_inner(|i| {
        i.state.target = Some(target.clone());
        i.state.player = "loading".into();
        i.state.error = None;
        i.state.audio_only
    });
    emit(&app);

    let picked = resolve(&app, &target, audio_only).await;
    if LOAD_SEQ.load(Ordering::Acquire) != seq {
        return;
    }
    let picked = match picked {
        Ok(p) => p,
        Err(message) => {
            log::warn!("[Cast] {}:{} not castable: {message}", target.provider, target.login);
            with_inner(|i| {
                i.state.player = "idle".into();
                i.state.error = Some(message);
            });
            emit(&app);
            return;
        }
    };
    log::info!(
        "[Cast] loading {}:{} at {} ({}, audio_only={audio_only})",
        target.provider,
        target.login,
        picked.label,
        picked.segment_format
    );

    let subtitle = match target.provider.as_str() {
        "kick" => format!("{} on Kick", target.name),
        _ => format!("{} on Twitch", target.name),
    };
    let args = crate::cast_plugin::LoadArgs {
        url: picked.url,
        segment_format: picked.segment_format.to_string(),
        audio_only,
        title: if target.title.is_empty() { target.name.clone() } else { target.title.clone() },
        subtitle,
        custom: serde_json::to_string(&target).unwrap_or_default(),
    };
    if let Err(e) = crate::cast_plugin::load(&app, args).await {
        log::warn!("[Cast] load call failed: {e}");
        with_inner(|i| i.state.error = Some("Couldn't send the stream to the TV.".into()));
        emit(&app);
    }
}

struct Picked {
    url: String,
    label: String,
    segment_format: &'static str,
}

async fn resolve(app: &AppHandle, target: &CastTarget, audio_only: bool) -> Result<Picked, String> {
    let (url, label) = match target.provider.as_str() {
        "twitch" => resolve_twitch(app, &target.login, audio_only).await?,
        "kick" => resolve_kick(&target.login, audio_only).await?,
        "youtube" => return Err(not_castable("YouTube")),
        "tiktok" => return Err(not_castable("TikTok")),
        other => return Err(not_castable(other)),
    };
    let segment_format = segment_format_of(&url).await;
    Ok(Picked { url, label, segment_format })
}

fn not_castable(platform: &str) -> String {
    format!("{platform} streams can't be cast yet. Mirror your screen to watch on the TV.")
}

async fn resolve_twitch(app: &AppHandle, login: &str, audio_only: bool) -> Result<(String, String), String> {
    let state = app.state::<AppState>();
    let (video, streamlink) = {
        let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
        (s.video_player.clone(), s.streamlink.clone())
    };
    let oauth = state.twitch_auth.get_token().await.ok();
    let core = tr::resolve_live_resilient(
        login,
        oauth.as_deref(),
        "best",
        streamlink.retry_streams,
        streamlink.stream_timeout,
    )
    .await;
    let relayed = crate::services::ad_bypass::resolve_detached(login, core.as_ref().ok(), &video).await;
    let (master, cap) = match (relayed, core) {
        (Some(r), _) => (r.master, Some(RELAYED_HEIGHT_CAP)),
        (None, Ok(c)) => (c.master, None),
        (None, Err(e)) => return Err(format!("Couldn't find {login}'s stream ({e}).")),
    };
    let variants = tr::parse_master(&master);
    let renditions: Vec<Rendition> = variants
        .iter()
        .map(|v| Rendition {
            name: &v.name,
            height: v.height,
            fps: v.fps,
            codecs: v.codecs.as_deref(),
            bandwidth: v.bandwidth,
        })
        .collect();
    let idx = pick_rendition(&renditions, audio_only, cap).ok_or_else(|| no_rendition(audio_only))?;
    Ok((variants[idx].url.clone(), variants[idx].name.clone()))
}

async fn resolve_kick(login: &str, audio_only: bool) -> Result<(String, String), String> {
    let source = crate::services::providers::registry()
        .await
        .get_source("kick")
        .ok_or_else(|| "Kick isn't available in this build.".to_string())?;
    let resolved = source
        .resolve_playback(CAST_STREAM_ID, login, "best")
        .await
        .map_err(|e| format!("Couldn't find {login}'s stream ({e})."))?;
    let renditions: Vec<Rendition> = resolved
        .qualities
        .iter()
        .map(|q| Rendition {
            name: &q.name,
            height: q.height,
            fps: q.fps,
            codecs: None,
            bandwidth: q.bandwidth,
        })
        .collect();
    let idx = pick_rendition(&renditions, audio_only, None).ok_or_else(|| no_rendition(audio_only))?;
    Ok((resolved.qualities[idx].url.clone(), resolved.qualities[idx].name.clone()))
}

fn no_rendition(audio_only: bool) -> String {
    if audio_only {
        "This stream has no audio-only version, and speakers can't play video.".to_string()
    } else {
        "This stream has no version the TV can play.".to_string()
    }
}

/// `"fmp4"` when the media playlist names an init segment, else `"ts"`. Only
/// the receiver's legacy player reads it; a failed fetch assumes TS, which is
/// what every H.264 rendition here has been.
async fn segment_format_of(url: &str) -> &'static str {
    let body = crate::services::http::client()
        .get(url)
        .timeout(Duration::from_secs(8))
        .send()
        .await;
    match body {
        Ok(r) => match r.text().await {
            Ok(text) if text.contains("#EXT-X-MAP") => "fmp4",
            _ => "ts",
        },
        Err(_) => "ts",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_events_parse() {
        let e: NativeEvent =
            serde_json::from_str(r#"{"kind":"session","state":"started","device":"TV","video_out":false}"#).unwrap();
        assert!(matches!(e, NativeEvent::Session { ref state, video_out: false, .. } if state == "started"));
        let e: NativeEvent = serde_json::from_str(
            r#"{"kind":"media","state":"playing","custom":{"provider":"kick","login":"x"}}"#,
        )
        .unwrap();
        assert!(matches!(e, NativeEvent::Media { custom: Some(_), .. }));
    }
}
