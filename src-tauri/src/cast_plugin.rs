// Google Cast on Android: the bridge between `services/cast_session.rs` and the
// Kotlin side (`CastPlugin.kt` + `CastRemote.kt`).
//
// Two directions, two mechanisms, both already in use in this app:
// - Rust to Kotlin through a Tauri Android plugin (`run_mobile_plugin`), the
//   same shape as `twitch_login_plugin.rs`. Plugin commands need an ACL grant
//   an app-local plugin does not get, so the page never calls the plugin
//   itself: it calls the app commands below, which forward.
// - Kotlin to Rust through a JNI export (`CastBridge.onEvent`), the same shape
//   as `android_notify.rs`. The SDK's session callbacks fire on the main thread
//   with no Tauri context, and the session outlives any one page.
#![cfg(target_os = "android")]

use crate::rt::{AppHandle, Rt};
use crate::services::cast_session::{self, CastState};
use jni::objects::{JObject, JString};
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct CastPluginState<R: Runtime>(pub PluginHandle<R>);

/// What `CastRemote.load` needs to put a stream on the receiver.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadArgs {
    /// The upstream media playlist (never the phone's localhost relay).
    pub url: String,
    /// "ts" | "fmp4", for the receiver's legacy HLS player.
    pub segment_format: String,
    pub audio_only: bool,
    pub title: String,
    pub subtitle: String,
    /// The cast target as JSON, carried in the load's `customData` so a resumed
    /// session can say what the receiver is playing.
    pub custom: String,
}

#[derive(Serialize)]
struct ControlArgs {
    action: String,
}

#[derive(Deserialize)]
struct OpenedResp {
    opened: bool,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("cast")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.streamnook", "CastPlugin")?;
            app.manage(CastPluginState(handle));
            Ok(())
        })
        .build()
}

/// Run one plugin method off the async runtime: `run_mobile_plugin` blocks
/// until Kotlin answers, and Kotlin answers from the main thread.
async fn call<T>(app: &AppHandle, method: &'static str, args: impl Serialize + Send + 'static) -> Result<T, String>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<CastPluginState<Rt>>();
        state.0.run_mobile_plugin::<T>(method, args).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct TvShowArgs {
    url: String,
}

/// TV screen mode: play `url` (the phone relay's playlist) full-frame on the
/// mirrored display.
pub async fn tv_show(app: &AppHandle, url: String) -> Result<(), String> {
    call::<serde_json::Value>(app, "tvShow", TvShowArgs { url }).await.map(|_| ())
}

/// TV screen mode off: the TV goes back to mirroring the phone.
pub async fn tv_hide(app: &AppHandle) -> Result<(), String> {
    call::<serde_json::Value>(app, "tvHide", ()).await.map(|_| ())
}

pub async fn load(app: &AppHandle, args: LoadArgs) -> Result<(), String> {
    call::<serde_json::Value>(app, "load", args).await.map(|_| ())
}

// ---- page commands ----------------------------------------------------------------

#[tauri::command]
pub fn cast_get_state() -> CastState {
    cast_session::state()
}

/// Open Google's device picker.
#[tauri::command]
pub async fn cast_open_picker(app: AppHandle) -> Result<(), String> {
    call::<serde_json::Value>(&app, "showPicker", ()).await.map(|_| ())
}

/// Transport for the casting card: "play" | "pause" | "stop" (ends the
/// session; the receiver closes its player).
#[tauri::command]
pub async fn cast_control(app: AppHandle, action: String) -> Result<(), String> {
    if !matches!(action.as_str(), "play" | "pause" | "stop") {
        return Err(format!("unknown cast action '{action}'"));
    }
    call::<serde_json::Value>(&app, "control", ControlArgs { action }).await.map(|_| ())
}

/// Open the system's screen-cast picker (mirroring is the system's job: apps
/// cannot drive Miracast). False when this phone's settings have no such page.
#[tauri::command]
pub async fn cast_open_mirroring(app: AppHandle) -> Result<bool, String> {
    call::<OpenedResp>(&app, "openMirroring", ()).await.map(|r| r.opened)
}

#[tauri::command]
pub fn tv_get_state() -> crate::services::tv_screen::TvState {
    crate::services::tv_screen::state()
}

/// The viewer's switch between TV screen mode and plain mirroring.
#[tauri::command]
pub fn tv_set_enabled(app: AppHandle, enabled: bool) {
    crate::services::tv_screen::set_enabled(&app, enabled);
}

/// Transport for the TV card: "play" | "pause".
#[tauri::command]
pub async fn tv_control(app: AppHandle, action: String) -> Result<(), String> {
    if !matches!(action.as_str(), "play" | "pause") {
        return Err(format!("unknown TV action '{action}'"));
    }
    call::<serde_json::Value>(&app, "tvControl", ControlArgs { action }).await.map(|_| ())
}

// ---- Kotlin to Rust ---------------------------------------------------------------

/// `app.streamnook.CastBridge.onEvent(json)`. `CastBridge` is a Kotlin `object`,
/// so the second argument is the singleton (see `android_notify.rs` for why a
/// companion object would never resolve).
#[no_mangle]
pub extern "system" fn Java_app_streamnook_CastBridge_onEvent<'local>(
    mut env: JNIEnv<'local>,
    _this: JObject<'local>,
    json: JString<'local>,
) {
    let json: String = match env.get_string(&json) {
        Ok(s) => s.into(),
        Err(_) => return,
    };
    // Never unwind across the JNI boundary.
    let _ = std::panic::catch_unwind(|| cast_session::on_native_event(json));
}
