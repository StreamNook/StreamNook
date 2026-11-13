//! The P2 commands: the log folder (desktop) and the log share (phones), and
//! the people the Whisper action may write to.

use crate::services::account_automations::local::WhisperPerson;

/// Open `logs/automations/` in the OS file manager. Registered everywhere
/// (one ACL file, D275); off desktop it answers a plain refusal.
#[tauri::command]
pub async fn automations_open_log_folder() -> Result<(), String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Err("Opening the folder works in the desktop app. Use Share log on your phone.".to_string())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let dir = crate::services::account_automations::logfile::real_dir()
            .ok_or_else(|| "The log folder isn't available".to_string())?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let program = if cfg!(target_os = "windows") {
            "explorer"
        } else if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        std::process::Command::new(program)
            .arg(dir.as_os_str())
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Hand the phone's automation logs over (the `export_logs` pattern,
/// `commands/logs.rs:190-246`): zipped into the cache, then iOS
/// `ios_ffi::share_file` or Android `twitch_login_plugin::share_file`.
/// Answers "shared" or "empty".
#[cfg(mobile)]
#[tauri::command]
pub async fn automations_share_log(app: crate::rt::AppHandle) -> Result<String, String> {
    use tauri::Manager;

    const MIME: &str = "application/zip";
    let dir = crate::services::account_automations::logfile::real_dir()
        .ok_or_else(|| "The log folder isn't available".to_string())?;
    let dest = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("shared-logs")
        .join("streamnook-automation-logs.zip");
    let (src, zip_dest) = (dir, dest.clone());
    let count = tauri::async_runtime::spawn_blocking(move || {
        crate::commands::logs::zip_logs(&src, &zip_dest)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok("empty".to_string());
    }
    let name = format!(
        "streamnook-automation-logs-{}.zip",
        chrono::Local::now().format("%Y-%m-%d-%H%M")
    );
    #[cfg(target_os = "ios")]
    crate::ios_ffi::share_file(&dest, MIME, &name);
    #[cfg(target_os = "android")]
    crate::twitch_login_plugin::share_file(&app, &dest, &name, MIME)?;
    Ok("shared".to_string())
}

/// The people the Whisper action's To picker lists: conversations the
/// whisper store says you have written to (`whisper_store::knows`), for the
/// signed-in account. Empty when no whisper account is followed.
#[tauri::command]
pub async fn automations_whisper_people() -> Result<Vec<WhisperPerson>, String> {
    let Some(engine) = crate::services::account_automations::engine() else {
        return Ok(Vec::new());
    };
    let Some(owner) = engine.main_id() else {
        return Ok(Vec::new());
    };
    Ok(engine.local.whisper_people(&owner))
}
