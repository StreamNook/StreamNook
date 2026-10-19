//! TV screen mode: while the phone mirrors to a TV, the stream plays full-frame
//! on the TV and the phone keeps chat.
//!
//! Mirroring alone shows the TV a picture of the whole phone, so a 16:9 stream
//! inside a ~20:9 phone screen fills about 65% of a 16:9 TV. Android lets an
//! app draw its own content on a mirrored display that carries
//! `FLAG_PRESENTATION` (an Android `Presentation`), at the TV's own size. Kotlin
//! (`TvScreen.kt`) watches for such a display and owns the Presentation and its
//! native player; this module decides WHAT it plays and WHEN, so the decision
//! survives the page and follows every stream start, restart and stop.
//!
//! The TV player plays the phone's own stream relay (the localhost playlist the
//! page would have played), so ad filtering, quality and every platform's relay
//! behave exactly as on the phone, and there is still one decoder: the page's
//! player is replaced by a card while the TV has the stream.
//!
//! Events:
//! - `tv-screen://state` the whole [`TvState`], on every change.
#![cfg(target_os = "android")]

use crate::rt::AppHandle;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Emitter;

pub const STATE_EVENT: &str = "tv-screen://state";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TvDisplay {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TvState {
    /// The mirrored display a stream can be drawn on, while there is one.
    pub display: Option<TvDisplay>,
    /// The viewer's choice: TV screen mode while mirroring (default), or plain
    /// mirroring of the phone. Not persisted; every mirroring session starts on.
    pub enabled: bool,
    /// The TV is playing the stream now (the page shows the remote card).
    pub active: bool,
    /// The TV player: "loading" | "playing" | "paused" | "buffering" | "idle".
    pub player: String,
    pub error: Option<String>,
}

struct Inner {
    state: TvState,
    /// The phone relay's playlist for the stream on screen, if it is HLS.
    local_url: Option<String>,
    /// What the TV was last told to play.
    shown_url: Option<String>,
}

static INNER: Mutex<Option<Inner>> = Mutex::new(None);

fn with_inner<R>(f: impl FnOnce(&mut Inner) -> R) -> R {
    let mut guard = INNER.lock().unwrap_or_else(|e| e.into_inner());
    let inner = guard.get_or_insert_with(|| Inner {
        state: TvState { display: None, enabled: true, active: false, player: "idle".into(), error: None },
        local_url: None,
        shown_url: None,
    });
    f(inner)
}

pub fn state() -> TvState {
    with_inner(|i| i.state.clone())
}

fn emit(app: &AppHandle) {
    let _ = app.emit(STATE_EVENT, state());
}

/// A playlist the TV player can take: the relay's HLS, or an adapter's local
/// HLS. A clip (a direct MP4) and a cast placeholder are not.
fn playable(url: &str) -> bool {
    (url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1")) && url.contains(".m3u8")
}

/// `start_stream` produced a stream (Some) or `stop_stream` ended it (None).
///
/// A stop is applied late: restarts and quality changes stop the stream and
/// start it again a moment later, and applying the stop at once flashed the
/// TV back to plain mirroring in between.
pub fn on_local_stream(url: Option<&str>) {
    let next = url.filter(|u| playable(u)).map(str::to_string);
    let stopped = next.is_none();
    with_inner(|i| i.local_url = next);
    let Some(app) = crate::services::cast_session::app() else { return };
    if !stopped {
        apply(&app);
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        if with_inner(|i| i.local_url.is_none()) {
            apply(&app);
        }
    });
}

/// From `TvScreen.kt`: a presentation display appeared, changed or went away.
pub(crate) fn on_display(app: &AppHandle, display: Option<TvDisplay>) {
    log::info!("[TvScreen] display {:?}", display);
    with_inner(|i| {
        if display.is_none() {
            // The next mirroring session starts in TV screen mode again.
            i.state.enabled = true;
        }
        i.state.display = display;
    });
    apply(app);
}

/// From `TvScreen.kt`: the TV player's state.
pub(crate) fn on_player(app: &AppHandle, player: String, error: Option<String>) {
    with_inner(|i| {
        i.state.player = player;
        i.state.error = error;
    });
    emit(app);
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    with_inner(|i| i.state.enabled = enabled);
    apply(app);
}

/// Bring the TV in line with the state: show the stream, switch it, or hand
/// the TV back to plain mirroring. A Cast session takes precedence: the
/// stream is on the Cast device, not on the phone's relay.
pub(crate) fn apply(app: &AppHandle) {
    let casting = crate::services::cast_session::state().phase == crate::services::cast_session::Phase::Casting;
    let (want, changed) = with_inner(|i| {
        let want = match (&i.state.display, i.state.enabled, casting) {
            (Some(_), true, false) if crate::services::cast_session::OFFERED => i.local_url.clone(),
            _ => None,
        };
        let changed = want != i.shown_url;
        i.shown_url = want.clone();
        i.state.active = want.is_some();
        if want.is_none() {
            i.state.player = "idle".into();
            i.state.error = None;
        } else if changed {
            i.state.player = "loading".into();
            i.state.error = None;
        }
        (want, changed)
    });
    emit(app);
    if !changed {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = match want {
            Some(url) => crate::cast_plugin::tv_show(&app, url).await,
            None => crate::cast_plugin::tv_hide(&app).await,
        };
        if let Err(e) = result {
            log::warn!("[TvScreen] {e}");
            with_inner(|i| i.state.error = Some("Couldn't show the stream on the TV.".into()));
            emit(&app);
        }
    });
}
