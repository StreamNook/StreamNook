//! Tauri commands backing the Plugins settings page and the consent flow.
//! Thin wrappers over plugin_host::PluginHost; errors surface as strings.

use serde_json::Value;
use tauri::State;

use crate::models::settings::AppState;
use crate::plugin_host::{install::IndexEntry, PluginInfo, SourceInfo};

#[tauri::command]
pub async fn plugins_list(state: State<'_, AppState>) -> Result<Vec<PluginInfo>, String> {
    Ok(state.plugin_host.list_installed().await)
}

#[tauri::command]
pub async fn plugins_sources(state: State<'_, AppState>) -> Result<Vec<SourceInfo>, String> {
    Ok(state.plugin_host.sources().await)
}

#[tauri::command]
pub async fn plugins_add_source(
    url: String,
    state: State<'_, AppState>,
) -> Result<SourceInfo, String> {
    state
        .plugin_host
        .add_source(&url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_remove_source(url: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .plugin_host
        .remove_source(&url)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_browse_source(
    url: String,
    state: State<'_, AppState>,
) -> Result<crate::plugin_host::SourceListing, String> {
    state
        .plugin_host
        .browse_source(&url)
        .await
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct InstallPreview {
    pub token: String,
    pub record: crate::plugin_host::registry::InstalledPlugin,
}

/// Downloads, verifies, and stages a plugin. Returns the consent-dialog
/// payload; nothing is registered until plugins_commit_install.
#[tauri::command]
pub async fn plugins_begin_install(
    source_url: String,
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<InstallPreview, String> {
    state
        .plugin_host
        .begin_install(&source_url, &plugin_id)
        .await
        .map(|(token, record)| InstallPreview { token, record })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_commit_install(
    token: String,
    state: State<'_, AppState>,
) -> Result<PluginInfo, String> {
    state
        .plugin_host
        .commit_install(&token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_cancel_install(
    token: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .cancel_install(&token)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_install_local(
    dir: String,
    state: State<'_, AppState>,
) -> Result<PluginInfo, String> {
    state
        .plugin_host
        .install_local(&dir)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_uninstall(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .uninstall(&plugin_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_set_enabled(
    plugin_id: String,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .set_enabled(&plugin_id, enabled)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_get_panel(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<Option<Value>, String> {
    Ok(state.plugin_host.get_panel(&plugin_id).await)
}

#[tauri::command]
pub async fn plugins_set_panel_values(
    plugin_id: String,
    values: Value,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .set_panel_values(&plugin_id, values)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_respond_consent(
    request_id: String,
    decision: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .respond_consent(&request_id, &decision)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_revoke_credential(
    plugin_id: String,
    kind: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .revoke_credential(&plugin_id, &kind)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_reset_credential_consent(
    plugin_id: String,
    kind: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .reset_credential_consent(&plugin_id, &kind)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_audit_log(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    Ok(state.plugin_host.audit_log(&plugin_id))
}

/// Marketplace detail page README fetch (presentation only).
#[tauri::command]
pub async fn plugins_fetch_readme(url: String) -> Result<String, String> {
    crate::plugin_host::install::fetch_readme(&url)
        .await
        .map_err(|e| e.to_string())
}

/// Invokes a named hook action on whichever plugin handles it. Core UI uses
/// this to delegate a control (e.g. start collecting a campaign) without knowing
/// which plugin backs it.
#[tauri::command]
pub async fn plugins_invoke_action(
    action: String,
    args: Value,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    state
        .plugin_host
        .invoke_action(&action, args)
        .await
        .map_err(|e| e.to_string())
}

/// The id of a running plugin that provides a feature, or null. Core UI uses
/// this to light up controls only when something backs them.
#[tauri::command]
pub async fn plugins_provides(
    feature: String,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    Ok(state.plugin_host.provides(&feature).await)
}

/// The bundled module source of an enabled ui plugin, read for the frontend
/// loader (see docs/plugins/UI_PLUGINS.md).
#[tauri::command]
pub async fn plugins_ui_bundle(
    plugin_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state
        .plugin_host
        .ui_bundle(&plugin_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn plugins_report_stream_event(
    kind: String,
    channel_id: Option<String>,
    login: Option<String>,
    display_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .plugin_host
        .report_stream_event(&kind, channel_id, login, display_name)
        .await
        .map_err(|e| e.to_string())
}
