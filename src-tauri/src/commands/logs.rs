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

/// Zip every non-empty file in `logs_dir` into `dest`, in name order. Returns
/// how many went in; 0 means there was nothing to send and no archive is left.
/// `dest` must sit outside `logs_dir`, or the archive would try to include
/// itself.
pub(crate) fn zip_logs(logs_dir: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<usize> {
    use std::io::Write;

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(logs_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.metadata().map(|m| m.is_file() && m.len() > 0).unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        let _ = std::fs::remove_file(dest);
        return Ok(0);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut zip = zip::ZipWriter::new(std::fs::File::create(dest)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for path in &files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("log");
        zip.start_file(name, options)?;
        std::io::copy(&mut std::fs::File::open(path)?, &mut zip)?;
    }
    zip.finish()?.flush()?;
    Ok(files.len())
}

/// Hand the phone's logs over. They live in the app's private data folder,
/// which no file manager can open without root, so they are zipped into the
/// cache and then saved to Downloads (`to = "downloads"`) or offered to the
/// share sheet (`to = "share"`). Answers "saved", "shared" or "empty". On
/// Android 9 and older, Downloads needs a storage permission the app does not
/// hold, so a save falls back to the share sheet and answers "shared".
#[cfg(target_os = "android")]
#[command]
pub async fn export_logs(app: tauri::AppHandle, to: String) -> Result<String, String> {
    use tauri::Manager;

    const MIME: &str = "application/zip";
    let logs_dir = crate::services::file_log::log_file_path()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or_else(|| "logs dir has no parent".to_string())?
        .to_path_buf();
    let dest = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("shared-logs")
        .join("streamnook-logs.zip");
    let zip_dest = dest.clone();
    let count = tauri::async_runtime::spawn_blocking(move || zip_logs(&logs_dir, &zip_dest))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            log::warn!("[Logs] could not package the logs: {}", e);
            e.to_string()
        })?;
    if count == 0 {
        return Ok("empty".to_string());
    }

    // Dated, so a second save sits beside the first instead of "(1)".
    let saved_name = format!("streamnook-logs-{}.zip", chrono::Local::now().format("%Y-%m-%d-%H%M"));
    let result = match to.as_str() {
        "downloads" => match crate::twitch_login_plugin::save_to_downloads(&app, &dest, &saved_name, MIME)? {
            true => "saved",
            false => {
                crate::twitch_login_plugin::share_file(&app, &dest, &saved_name, MIME)?;
                "shared"
            }
        },
        "share" => {
            crate::twitch_login_plugin::share_file(&app, &dest, "streamnook-logs.zip", MIME)?;
            "shared"
        }
        other => return Err(format!("unknown log destination: {}", other)),
    };
    log::info!("[Logs] exported {} file(s): {}", count, result);
    Ok(result.to_string())
}

#[cfg(test)]
mod tests {
    use super::zip_logs;

    #[test]
    fn zip_logs_packs_non_empty_files_and_skips_the_rest() {
        let root = std::env::temp_dir().join(format!("sn-zip-logs-{}", std::process::id()));
        let logs = root.join("logs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(logs.join("nested")).unwrap();
        std::fs::write(logs.join("streamnook.log"), "line one\nline two\n").unwrap();
        std::fs::write(logs.join("errors.log"), "boom\n").unwrap();
        std::fs::write(logs.join("empty.log"), "").unwrap();
        let dest = root.join("out").join("logs.zip");

        assert_eq!(zip_logs(&logs, &dest).unwrap(), 2);
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["errors.log", "streamnook.log"]);
        let mut body = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("streamnook.log").unwrap(), &mut body).unwrap();
        assert_eq!(body, "line one\nline two\n");

        // Nothing logged: no archive, and a stale one from before is removed.
        for f in ["streamnook.log", "errors.log"] {
            std::fs::write(logs.join(f), "").unwrap();
        }
        assert_eq!(zip_logs(&logs, &dest).unwrap(), 0);
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
