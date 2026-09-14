//! Tauri boundary for the Rust chat rule engine and history ring. Thin:
//! validate intent, call the service, return a compact result.
//!
//! Every command here has its `generate_handler!` entry in main.rs and its
//! line in permissions/app-commands.toml (acl_parity test enforces it).

use crate::services::chat_history::{ChatHistory, HistoryStats, SearchHit};
use crate::services::chat_rules::{ChatRules, RuleError};
use serde::Serialize;

/// Search the Rust-owned history ring. `channel` is the slice key the pane
/// already uses (bare Twitch login or `provider:channel`); None searches
/// every joined channel. At most `limit` (default 100, max 500) newest-first
/// compact hits cross IPC, never the corpus.
#[tauri::command]
pub async fn search_chat(
    channel: Option<String>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let limit = limit.unwrap_or(100);
    tokio::task::spawn_blocking(move || ChatHistory::search(channel.as_deref(), &query, limit))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
pub struct FilterValidation {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Variables the expression uses that StreamNook cannot populate yet.
    pub unsupported: Vec<String>,
}

/// Chatterino-syntax filter check for the editor, in the dialect that runs.
#[tauri::command]
pub async fn validate_chat_filter(expr: String) -> Result<FilterValidation, String> {
    Ok(match ChatRules::validate_filter(&expr) {
        Ok(unsupported) => FilterValidation {
            ok: true,
            error: None,
            unsupported,
        },
        Err(e) => FilterValidation {
            ok: false,
            error: Some(e),
            unsupported: Vec::new(),
        },
    })
}

/// Regex check for highlight and ignore phrases (Rust `regex` dialect: no
/// lookaround, no backreferences). Returns the compile error text or null.
#[tauri::command]
pub async fn validate_chat_phrase(
    pattern: String,
    is_regex: bool,
    whole_word: Option<bool>,
    case_sensitive: Option<bool>,
) -> Result<Option<String>, String> {
    Ok(ChatRules::validate_phrase(
        &pattern,
        is_regex,
        whole_word.unwrap_or(false),
        case_sensitive.unwrap_or(false),
    )
    .err())
}

/// Rules that failed to compile at the last refresh, for the settings UI.
#[tauri::command]
pub async fn get_chat_rule_errors() -> Result<Vec<RuleError>, String> {
    Ok(ChatRules::snapshot().errors.clone())
}

/// Ring occupancy for the resource log and the settings tooltip.
#[tauri::command]
pub async fn get_chat_history_stats() -> Result<HistoryStats, String> {
    Ok(ChatHistory::stats())
}
