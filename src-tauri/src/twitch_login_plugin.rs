// Android in-app Twitch login. Tauri 2 cannot open a second WebView on Android
// (the child-webview API is desktop-only), so login is presented by a native
// Kotlin WebView overlay registered as an Android plugin. The Kotlin class
// `app.streamnook.TwitchLoginPlugin` exposes openLogin/closeLogin/getCookies.
//
// Plugin commands (`plugin:twitch-login|...`) always require an ACL grant, which
// app-local plugins don't get for free. So instead of invoking the Kotlin plugin
// directly from JS, we forward through regular app commands (always allowed from
// the local origin) that call the stored PluginHandle.
#![cfg(target_os = "android")]

use serde::{Deserialize, Serialize};
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime};

pub struct TwitchLoginState<R: Runtime>(pub PluginHandle<R>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenLoginArgs {
    url: String,
    /// localStorage key the overlay should watch for. See the Kotlin side.
    #[serde(skip_serializing_if = "Option::is_none")]
    watch_storage_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

#[derive(Deserialize)]
struct CookiesResp {
    cookies: String,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("twitch-login")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.streamnook", "TwitchLoginPlugin")?;
            app.manage(TwitchLoginState(handle));
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn open_mobile_login<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    watch_storage_key: Option<String>,
    title: Option<String>,
) -> Result<(), String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<serde_json::Value>(
            "openLogin",
            OpenLoginArgs {
                url,
                watch_storage_key,
                title,
            },
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_mobile_login<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<serde_json::Value>("closeLogin", ())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_mobile_login_cookies<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<CookiesResp>("getCookies", ())
        .map(|r| r.cookies)
        .map_err(|e| e.to_string())
}
