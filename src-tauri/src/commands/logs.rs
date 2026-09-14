use crate::services::log_service::{ActivityEntry, LogEntry, LogLevel, LogService};
use tauri::command;

/// Put a frontend diagnostic into the SAME log the backend writes.
///
/// `LogService` keeps its own in-app buffer and does not go through the `log`
/// crate, and the frontend `Logger` writes to the devtools console (with info/debug
/// off by default). So a frontend trace was invisible in the log file people
/// actually read and paste. This is the one-line bridge for that.
#[command]
pub fn log_frontend_diag(message: String) {
    log::info!("[frontend] {}", message);
}

#[command]
pub async fn log_message(
    level: String,
    category: String,
    message: String,
    data: Option<serde_json::Value>,
) -> Result<(), String> {
    let log_level = match level.to_lowercase().as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    // Mirror into the log crate so frontend lines land in streamnook.log,
    // interleaved chronologically with the backend's. LogService keeps its
    // separate in-app ring buffer + crash-log role below.
    let detail = data.as_ref().map(|d| match d {
        // The frontend hands us JSON.stringify output, i.e. a JSON *string*.
        // Displaying the Value would quote and escape it a second time.
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    });
    let text = match &detail {
        Some(d) => format!("[frontend:{}] {} | {}", category, message, d),
        None => format!("[frontend:{}] {}", category, message),
    };
    match log_level {
        LogLevel::Error => log::error!(target: "frontend", "{}", text),
        LogLevel::Warn => log::warn!(target: "frontend", "{}", text),
        LogLevel::Info => log::info!(target: "frontend", "{}", text),
        LogLevel::Debug => log::debug!(target: "frontend", "{}", text),
    }

    LogService::log_message(log_level, category, message, data)
        .await
        .map_err(|e| e.to_string())
}

/// One queued frontend console line, as batched by logService's forward queue.
#[derive(serde::Deserialize)]
pub struct FrontendLogEntry {
    pub level: String,
    pub category: String,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// Batched variant of log_message: a console warn/error storm (e.g. a player
/// library during buffer degradation) used to cost one IPC round trip per
/// line, exactly when the main thread was already stressed.
#[command]
pub async fn log_messages_batch(entries: Vec<FrontendLogEntry>) -> Result<(), String> {
    for e in entries {
        log_message(e.level, e.category, e.message, e.data).await?;
    }
    Ok(())
}

#[command]
pub async fn track_activity(action: String) -> Result<(), String> {
    LogService::track_activity(action)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_recent_logs(limit: Option<usize>) -> Result<Vec<LogEntry>, String> {
    LogService::get_recent_logs(limit)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_logs_by_level(level: String) -> Result<Vec<LogEntry>, String> {
    let log_level = match level.to_lowercase().as_str() {
        "debug" => LogLevel::Debug,
        "info" => LogLevel::Info,
        "warn" => LogLevel::Warn,
        "error" => LogLevel::Error,
        _ => LogLevel::Info,
    };

    LogService::get_logs_by_level(log_level)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_recent_activity() -> Result<Vec<ActivityEntry>, String> {
    LogService::get_recent_activity()
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn clear_logs() -> Result<(), String> {
    LogService::clear_logs().await.map_err(|e| e.to_string())
}

/// Open the logs folder (streamnook.log, errors.log) in the OS file manager.
/// Local paths cannot go through the shell plugin (its `open` scope only allows
/// http/mailto/tel URLs), so this launches the platform file manager directly,
/// same as open_universal_cache_folder.
#[command]
pub async fn open_logs_folder() -> Result<(), String> {
    let file = crate::services::file_log::log_file_path().map_err(|e| e.to_string())?;
    let dir = file
        .parent()
        .ok_or_else(|| "logs dir has no parent".to_string())?
        .to_path_buf();
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
