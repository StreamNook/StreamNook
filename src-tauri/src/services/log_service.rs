use anyhow::Result;
use chrono::{DateTime, Utc};
use lazy_static::lazy_static;
use log::error;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::services::cache_service;

const MAX_LOGS: usize = 500;
const MAX_ACTIVITY_HISTORY: usize = 15;
// Rotate the local crash log once it grows past ~1 MB so it can't balloon unbounded.
const MAX_CRASH_LOG_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub category: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityEntry {
    pub timestamp: String,
    pub action: String,
}

struct LogState {
    logs: VecDeque<LogEntry>,
    activity_history: VecDeque<ActivityEntry>,
}

lazy_static! {
    static ref LOG_STATE: Arc<Mutex<LogState>> = Arc::new(Mutex::new(LogState {
        logs: VecDeque::with_capacity(MAX_LOGS),
        activity_history: VecDeque::with_capacity(MAX_ACTIVITY_HISTORY),
    }));
}

pub struct LogService;

impl LogService {
    /// Add a log entry to the ring buffer
    pub async fn log_message(
        level: LogLevel,
        category: String,
        message: String,
        data: Option<serde_json::Value>,
    ) -> Result<()> {
        let entry = LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            level: level.clone(),
            category,
            message,
            data,
        };

        let mut state = LOG_STATE.lock().await;

        // Add to ring buffer
        if state.logs.len() >= MAX_LOGS {
            state.logs.pop_front();
        }
        state.logs.push_back(entry.clone());

        // Persist genuine errors to the local crash log, skipping benign noise
        // (HLS buffer hiccups, handled React boundary errors, CDN blips, etc.).
        // This file stays on the user's machine and is never sent anywhere.
        if matches!(level, LogLevel::Error) && !Self::should_ignore_error(&entry) {
            // Pull recent warn/error breadcrumbs (excluding this entry) and the
            // local activity history so the crash log carries some context.
            let breadcrumbs: Vec<LogEntry> = state
                .logs
                .iter()
                .rev()
                .skip(1)
                .filter(|l| matches!(l.level, LogLevel::Warn | LogLevel::Error))
                .take(15)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let activity: Vec<ActivityEntry> = state.activity_history.iter().cloned().collect();
            drop(state); // Release lock before file I/O

            tokio::spawn(async move {
                if let Err(e) = LogService::append_to_crash_log(entry, breadcrumbs, activity).await
                {
                    error!("[LogService] Failed to write crash log: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Track user activity for error context
    pub async fn track_activity(action: String) -> Result<()> {
        let mut state = LOG_STATE.lock().await;

        let entry = ActivityEntry {
            timestamp: Utc::now().to_rfc3339(),
            action: action.chars().take(100).collect(), // Limit action length
        };

        if state.activity_history.len() >= MAX_ACTIVITY_HISTORY {
            state.activity_history.pop_front();
        }
        state.activity_history.push_back(entry);

        Ok(())
    }

    /// Get recent logs
    pub async fn get_recent_logs(limit: Option<usize>) -> Result<Vec<LogEntry>> {
        let state = LOG_STATE.lock().await;
        let limit = limit.unwrap_or(100);

        Ok(state
            .logs
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect())
    }

    /// Get logs by level
    pub async fn get_logs_by_level(level: LogLevel) -> Result<Vec<LogEntry>> {
        let state = LOG_STATE.lock().await;

        let min_level = match level {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
        };

        Ok(state
            .logs
            .iter()
            .filter(|log| {
                let log_level = match log.level {
                    LogLevel::Debug => 0,
                    LogLevel::Info => 1,
                    LogLevel::Warn => 2,
                    LogLevel::Error => 3,
                };
                log_level >= min_level
            })
            .cloned()
            .collect())
    }

    /// Get recent activity
    pub async fn get_recent_activity() -> Result<Vec<ActivityEntry>> {
        let state = LOG_STATE.lock().await;
        Ok(state.activity_history.iter().cloned().collect())
    }

    /// Clear all logs
    pub async fn clear_logs() -> Result<()> {
        let mut state = LOG_STATE.lock().await;
        state.logs.clear();
        Ok(())
    }

    /// Check if error should be ignored (benign/noise errors)
    fn should_ignore_error(entry: &LogEntry) -> bool {
        let ignored_patterns = [
            // App lifecycle / React errors
            "Couldn't find callback id",
            "This might happen when the app is reloaded",
            "ResizeObserver loop",
            "Non-Error promise rejection",
            "Failed to load resource.*favicon",
            "ERR_FILE_NOT_FOUND.*blob:",
            "Error caught and handled by boundary",
            "The above error occurred in the <TitleBar> component",
            "The above error occurred in the <DynamicIsland> component",
            // External resource / CDN errors
            "Tracking Prevention blocked",
            "cdn.jsdelivr.net",
            "emoji-datasource",
            // Service-specific noise
            "BadgePolling.*invoke",
            "Badge NOT FOUND",
            // HLS streaming non-fatal errors (expected during live streaming)
            "bufferStalledError",
            "bufferNudgeOnStall",
            "fragParsingError",
            "fragLoadError",
            "levelLoadError",
            "[HLS] Buffer stalled",
            "[HLS] Non-fatal error",
            "[HLS] Error.*fatal.*false",
            "manifestLoadError.*fatal.*false",
        ];

        let full_message = format!(
            "{} {} {}",
            entry.category,
            entry.message,
            entry.data.as_ref().map_or(String::new(), |d| d.to_string())
        );

        ignored_patterns
            .iter()
            .any(|pattern| full_message.contains(pattern))
    }

    /// Resolve the path to the local crash log (`<app_data>/logs/errors.log`).
    fn crash_log_path() -> Result<std::path::PathBuf> {
        let logs_dir = cache_service::get_app_data_dir()?.join("logs");
        std::fs::create_dir_all(&logs_dir)?;
        Ok(logs_dir.join("errors.log"))
    }

    /// Append a formatted error report (with recent breadcrumbs and local
    /// activity) to the crash log. Nothing here leaves the user's machine.
    async fn append_to_crash_log(
        error_entry: LogEntry,
        breadcrumbs: Vec<LogEntry>,
        activity: Vec<ActivityEntry>,
    ) -> Result<()> {
        let path = Self::crash_log_path()?;

        // Rotate to errors.log.old once the active log gets too large.
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if meta.len() > MAX_CRASH_LOG_BYTES {
                let _ = tokio::fs::rename(&path, path.with_file_name("errors.log.old")).await;
            }
        }

        let mut block = String::new();
        block.push_str(&format!(
            "\n========== {} ==========\n",
            error_entry.timestamp
        ));
        block.push_str(&format!(
            "App {} on {}\n",
            crate::services::client_identity::build_version(),
            std::env::consts::OS
        ));
        block.push_str(&format!(
            "ERROR [{}] {}{}\n",
            error_entry.category,
            error_entry.message,
            error_entry
                .data
                .as_ref()
                .map_or(String::new(), |d| format!(" | {}", d))
        ));

        if !breadcrumbs.is_empty() {
            block.push_str("\n-- Recent warnings/errors --\n");
            for l in &breadcrumbs {
                let time = l
                    .timestamp
                    .split('T')
                    .nth(1)
                    .and_then(|t| t.split('.').next())
                    .unwrap_or("");
                block.push_str(&format!(
                    "[{}] [{}] [{}] {}\n",
                    time,
                    l.level.to_string().to_uppercase(),
                    l.category,
                    l.message
                ));
            }
        }

        if !activity.is_empty() {
            block.push_str("\n-- Recent activity --\n");
            for a in &activity {
                let time = DateTime::parse_from_rfc3339(&a.timestamp)
                    .map(|d| d.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|_| "??:??:??".to_string());
                block.push_str(&format!("{} -> {}\n", time, a.action));
            }
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;
        file.write_all(block.as_bytes()).await?;
        Ok(())
    }
}
