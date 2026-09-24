//! MultiChat Activity feed history (see services/activity_history_service.rs).

use crate::services::activity_history_service as history;
use serde_json::Value;
use tauri::AppHandle;

#[tauri::command]
pub async fn activity_load(app: AppHandle) -> Result<Vec<Value>, String> {
    history::load(&app)
}

#[tauri::command]
pub async fn activity_append(app: AppHandle, event: Value) -> Result<history::Appended, String> {
    history::append(&app, event)
}

#[tauri::command]
pub async fn activity_import(app: AppHandle, events: Vec<Value>) -> Result<(), String> {
    history::import(&app, events)
}

#[tauri::command]
pub async fn activity_purge(app: AppHandle, source_keys: Vec<String>) -> Result<(), String> {
    history::purge(&app, source_keys)
}

#[tauri::command]
pub async fn activity_clear(app: AppHandle) -> Result<(), String> {
    history::clear(&app)
}
