// Android text to speech: the bridge to `SpeechPlugin.kt` for automations'
// Speak action and `/tts`.
//
// Rust to Kotlin through a Tauri Android plugin (`run_mobile_plugin`), the
// same shape as `cast_plugin.rs`. Plugin commands need an ACL grant an
// app-local plugin does not get, so the page never calls the plugin itself:
// it calls `speak_android` below, which forwards. The plugin never requests
// audio focus and queues a line until its engine is ready; it answers
// `{ spoken }`, and `false` (no speech engine, no voice, a failed call) is the
// page's cue to show the text instead. Nothing is ever posted to chat.
#![cfg(target_os = "android")]

use crate::rt::{AppHandle, Rt};
use crate::services::account_automations::speak::speakable;
use serde::{Deserialize, Serialize};
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

pub struct SpeechPluginState<R: Runtime>(pub PluginHandle<R>);

#[derive(Serialize)]
struct SpeakArgs {
    text: String,
}

/// What the plugin, and `speak_android`, answer.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Spoken {
    pub spoken: bool,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("speech")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.streamnook", "SpeechPlugin")?;
            app.manage(SpeechPluginState(handle));
            Ok(())
        })
        .build()
}

/// Speak one line. Every failure is `{ spoken: false }`: the caller shows the
/// text. `run_mobile_plugin` blocks until Kotlin answers (from the main
/// thread), so it runs off the async runtime.
pub async fn speak(app: &AppHandle, text: String) -> Spoken {
    let Some(text) = speakable(&text) else {
        return Spoken { spoken: false };
    };
    let app = app.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<SpeechPluginState<Rt>>();
        state
            .0
            .run_mobile_plugin::<Spoken>("speak", SpeakArgs { text })
            .map_err(|e| e.to_string())
    })
    .await;
    match answer {
        Ok(Ok(spoken)) => spoken,
        Ok(Err(e)) => {
            log::debug!("[Speech] the plugin failed: {e}");
            Spoken { spoken: false }
        }
        Err(e) => {
            log::debug!("[Speech] the call did not finish: {e}");
            Spoken { spoken: false }
        }
    }
}

/// `/tts` and the Speak action on Android. Android-only: it is in
/// `mobile-commands.toml` and nowhere else, and outside `ios_invoke_parity`.
#[tauri::command]
pub async fn speak_android(app: AppHandle, text: String) -> Spoken {
    speak(&app, text).await
}
