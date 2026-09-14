//! Capability enforcement and the credential broker. Every plugin-to-host
//! call lands here; anything not granted is denied. Credential handovers
//! require first-use consent and are written to the plugin's audit log.

use log::debug;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::oneshot;
use tokio::time::timeout;

use super::process::RpcErr;
use super::registry::{self, InstalledPlugin};
use super::{ConsentDecision, HostInner};
use crate::models::settings::AppState;
use crate::services::drops_auth_service::DropsAuthService;
use crate::services::twitch_service::TwitchService;

/// Seconds the first credential request may block on the consent prompt.
const CONSENT_TIMEOUT_SECS: u64 = 120;

pub async fn handle_host_method(
    host: &Arc<HostInner>,
    record: &InstalledPlugin,
    method: &str,
    params: Value,
) -> Result<Value, RpcErr> {
    match method {
        "get_followed_live" => {
            require_method(record, "get_followed_live")?;
            let channels = fetch_followed_live(host)
                .await
                .map_err(|e| RpcErr::internal(&e))?;
            Ok(json!({ "channels": channels }))
        }
        "set_upstream" => {
            require_method(record, "set_upstream")?;
            let stream_id = params
                .get("stream_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcErr::invalid_params("stream_id is required"))?;
            let playlist_url = params
                .get("playlist_url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcErr::invalid_params("playlist_url is required"))?;
            if !playlist_url.starts_with("http://") && !playlist_url.starts_with("https://") {
                return Err(RpcErr::invalid_params("playlist_url must be an http(s) URL"));
            }
            // Route to the matching relay session: the solo player's session
            // (stream id "solo") or a MultiNook tile by its id. The swap keeps
            // the local port and tells the player to reload onto it.
            if stream_id == crate::services::stream_server::SOLO_STREAM_ID {
                if !crate::services::stream_server::solo_session_active() {
                    return Err(RpcErr::unknown_stream(stream_id));
                }
                crate::services::stream_server::swap_upstream(playlist_url.to_string())
                    .await
                    .map_err(|e| RpcErr::internal(&e.to_string()))?;
            } else {
                // MultiNook tiles are desktop-only; on mobile any non-solo id is
                // unknown.
                #[cfg(desktop)]
                {
                    if crate::services::multi_nook_server::MultiNookServer::get_port(stream_id)
                        .await
                        .is_some()
                    {
                        crate::services::multi_nook_server::MultiNookServer::swap_upstream(
                            stream_id,
                            playlist_url.to_string(),
                        )
                        .await
                        .map_err(|e| RpcErr::internal(&e.to_string()))?;
                    } else {
                        return Err(RpcErr::unknown_stream(stream_id));
                    }
                }
                #[cfg(not(desktop))]
                return Err(RpcErr::unknown_stream(stream_id));
            }
            debug!(
                "[PluginHost] {} swapped the upstream for stream '{}'",
                record.id, stream_id
            );
            Ok(json!({}))
        }
        "get_credential" => {
            let kind = params
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcErr::invalid_params("kind is required"))?;
            get_credential(host, record, kind).await
        }
        "notify" => {
            require_method(record, "notify")?;
            let level = params.get("level").and_then(|v| v.as_str()).unwrap_or("info");
            if !matches!(level, "info" | "warning" | "error") {
                return Err(RpcErr::invalid_params("level must be info, warning, or error"));
            }
            let message = params
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or_else(|| RpcErr::invalid_params("message is required"))?;
            check_notify_rate(host, &record.id).await?;
            let _ = host.app.emit(
                "plugin://notify",
                json!({
                    "plugin_id": record.id,
                    "plugin_name": record.name,
                    "level": level,
                    "message": message,
                }),
            );
            Ok(json!({}))
        }
        "register_panel" => {
            require_ui_panel(record)?;
            let schema = params
                .get("schema")
                .cloned()
                .ok_or_else(|| RpcErr::invalid_params("schema is required"))?;
            validate_panel_schema(&schema)?;
            let path = registry::panel_schema_path(&record.id)
                .map_err(|e| RpcErr::internal(&e.to_string()))?;
            registry::write_json_file(&path, &schema)
                .map_err(|e| RpcErr::internal(&e.to_string()))?;
            let _ = host.app.emit(
                "plugin://panels-changed",
                json!({ "plugin_id": record.id }),
            );
            Ok(json!({}))
        }
        "get_panel_values" => {
            require_ui_panel(record)?;
            let path = registry::panel_values_path(&record.id)
                .map_err(|e| RpcErr::internal(&e.to_string()))?;
            let values = registry::read_json_file(&path).unwrap_or_else(|| json!({}));
            Ok(json!({ "values": values }))
        }
        other => Err(RpcErr::method_not_found(other)),
    }
}

fn require_method(record: &InstalledPlugin, method: &str) -> Result<(), RpcErr> {
    if record.granted.host_methods.iter().any(|m| m == method) {
        Ok(())
    } else {
        Err(RpcErr::capability_denied(&format!(
            "host method '{method}' is not granted"
        )))
    }
}

fn require_ui_panel(record: &InstalledPlugin) -> Result<(), RpcErr> {
    if record.granted.ui.iter().any(|u| u == "panel") {
        Ok(())
    } else {
        Err(RpcErr::capability_denied("ui capability 'panel' is not granted"))
    }
}

/// Burst of 3, then at most one notify per 10 seconds.
async fn check_notify_rate(host: &Arc<HostInner>, plugin_id: &str) -> Result<(), RpcErr> {
    let mut stamps = host.notify_stamps.lock().await;
    let entry = stamps.entry(plugin_id.to_string()).or_default();
    let window = Duration::from_secs(10);
    entry.retain(|t: &Instant| t.elapsed() < window);
    if entry.len() >= 3 {
        let oldest = entry.iter().min().cloned();
        let retry = oldest
            .map(|t| window.saturating_sub(t.elapsed()).as_millis() as u64)
            .unwrap_or(10_000);
        return Err(RpcErr::rate_limited(retry.max(1)));
    }
    entry.push(Instant::now());
    Ok(())
}

/// The credential broker: manifest grant, then consent state, then handover,
/// then audit. The only path a token ever crosses the plugin boundary on.
async fn get_credential(
    host: &Arc<HostInner>,
    record: &InstalledPlugin,
    kind: &str,
) -> Result<Value, RpcErr> {
    if !record.granted.credentials.iter().any(|c| c == kind) {
        return Err(RpcErr::capability_denied(&format!(
            "credential kind '{kind}' is not granted"
        )));
    }

    // Consent state: ask (default), always, revoked.
    let consent_state = {
        let registry = host.registry.lock().await;
        registry
            .plugins
            .iter()
            .find(|p| p.id == record.id)
            .and_then(|p| p.credential_consent.get(kind).cloned())
            .unwrap_or_else(|| "ask".to_string())
    };

    match consent_state.as_str() {
        "revoked" => return Err(RpcErr::consent_denied()),
        "always" => {}
        _ => {
            // Prompt the user and block on the answer (or time out as deny).
            let request_id = uuid::Uuid::new_v4().to_string();
            let (tx, rx) = oneshot::channel::<ConsentDecision>();
            host.consent_pending
                .lock()
                .await
                .insert(request_id.clone(), tx);
            let _ = host.app.emit(
                "plugin://consent-request",
                json!({
                    "request_id": request_id,
                    "plugin_id": record.id,
                    "plugin_name": record.name,
                    "kind": kind,
                }),
            );
            let decision = timeout(Duration::from_secs(CONSENT_TIMEOUT_SECS), rx).await;
            // Always clear the pending slot (it resolves to a no-op if the
            // responder already removed it).
            host.consent_pending.lock().await.remove(&request_id);
            match decision {
                Ok(Ok(ConsentDecision::Allow)) => {}
                Ok(Ok(ConsentDecision::Always)) => {
                    let _ = host
                        .set_credential_consent(&record.id, kind, "always")
                        .await;
                }
                _ => return Err(RpcErr::consent_denied()),
            }
        }
    }

    match kind {
        "twitch.android" => {
            let token = DropsAuthService::get_token()
                .await
                .map_err(|e| RpcErr::credential_unavailable(&e.to_string()))?;
            registry::audit_append(
                &record.id,
                "credential handover kind=twitch.android",
            );
            debug!(
                "[PluginHost] credential twitch.android handed to {}",
                record.id
            );
            Ok(json!({
                "kind": "twitch.android",
                "token": token,
                "client_id": env!("TWITCH_ANDROID_CLIENT_ID"),
                "user_id": Value::Null,
                "device_id": Value::Null,
                "expires_at": Value::Null,
            }))
        }
        other => Err(RpcErr::credential_unavailable(&format!(
            "unknown credential kind '{other}'"
        ))),
    }
}

/// Fetches the live followed channels and maps them to the protocol's
/// channel object shape.
pub async fn fetch_followed_live(host: &Arc<HostInner>) -> Result<Vec<Value>, String> {
    let state = host.app.state::<AppState>();
    let streams = TwitchService::get_followed_streams(&state)
        .await
        .map_err(|e| e.to_string())?;
    // Which of these live channels the user has favorited (hearted). Favorites
    // are Twitch user ids, the same id as a stream's user_id. The plugin uses
    // this to offer a favorites mode without shipping the list itself.
    let favorites: std::collections::HashSet<String> = state
        .settings
        .lock()
        .map(|s| s.favorite_streamers.iter().cloned().collect())
        .unwrap_or_default();
    Ok(streams
        .iter()
        .map(|s| {
            json!({
                "channel_id": s.user_id,
                "login": s.user_login,
                "display_name": s.user_name,
                "game_id": if s.game_id.is_empty() { Value::Null } else { json!(s.game_id) },
                "game_name": if s.game_name.is_empty() { Value::Null } else { json!(s.game_name) },
                "started_at": if s.started_at.is_empty() { Value::Null } else { json!(s.started_at) },
                "viewer_count": s.viewer_count,
                "favorite": favorites.contains(&s.user_id),
            })
        })
        .collect())
}

/// Structural validation of a host-rendered panel schema.
fn validate_panel_schema(schema: &Value) -> Result<(), RpcErr> {
    let title = schema
        .get("title")
        .and_then(|t| t.as_str())
        .ok_or_else(|| RpcErr::invalid_params("panel schema needs a title"))?;
    if title.is_empty() || title.chars().count() > 60 {
        return Err(RpcErr::invalid_params("panel title must be 1 to 60 characters"));
    }
    let sections = schema
        .get("sections")
        .and_then(|s| s.as_array())
        .ok_or_else(|| RpcErr::invalid_params("panel schema needs a sections array"))?;
    if sections.len() > 8 {
        return Err(RpcErr::invalid_params("panel schema allows at most 8 sections"));
    }
    for section in sections {
        let fields = section
            .get("fields")
            .and_then(|f| f.as_array())
            .ok_or_else(|| RpcErr::invalid_params("every section needs a fields array"))?;
        if fields.len() > 24 {
            return Err(RpcErr::invalid_params("a section allows at most 24 fields"));
        }
        for field in fields {
            let key = field
                .get("key")
                .and_then(|k| k.as_str())
                .ok_or_else(|| RpcErr::invalid_params("every field needs a key"))?;
            let key_ok = !key.is_empty()
                && key.len() <= 40
                && key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !key_ok {
                return Err(RpcErr::invalid_params(
                    "field keys are lowercase a-z, 0-9, and underscore, max 40 chars",
                ));
            }
            let field_type = field
                .get("type")
                .and_then(|t| t.as_str())
                .ok_or_else(|| RpcErr::invalid_params("every field needs a type"))?;
            match field_type {
                "toggle" | "number" | "text" | "string_list" | "channel_list" | "slider" => {}
                "select" => {
                    let has_options = field
                        .get("options")
                        .and_then(|o| o.as_array())
                        .map(|a| !a.is_empty())
                        .unwrap_or(false);
                    if !has_options {
                        return Err(RpcErr::invalid_params("select fields need options"));
                    }
                }
                other => {
                    return Err(RpcErr::invalid_params(&format!(
                        "unknown field type '{other}'"
                    )))
                }
            }
            if field.get("label").and_then(|l| l.as_str()).is_none() {
                return Err(RpcErr::invalid_params("every field needs a label"));
            }
        }
    }
    Ok(())
}

/// `set_status` notification: the plugin pushes a value into a named status
/// slot it declared. The host forwards it to the frontend as a `plugin://status`
/// event for whatever UI region owns that slot. Slots the plugin did not
/// declare are ignored.
pub fn handle_set_status(host: &Arc<HostInner>, record: &InstalledPlugin, params: &Value) {
    let Some(slot) = params.get("slot").and_then(|s| s.as_str()) else {
        return;
    };
    if !record.granted.status.iter().any(|s| s == slot) {
        return;
    }
    let value = params.get("value").cloned().unwrap_or(Value::Null);
    let _ = host.app.emit(
        "plugin://status",
        json!({ "plugin_id": record.id, "slot": slot, "value": value }),
    );
}

/// `log` notification from a plugin: append to its log file, never answered.
pub fn handle_log_notification(record: &InstalledPlugin, params: &Value) {
    let level = params.get("level").and_then(|v| v.as_str()).unwrap_or("info");
    let message = params.get("message").and_then(|v| v.as_str()).unwrap_or("");
    append_plugin_log(&record.id, level, message);
}

/// Appends one line to the plugin's own log file (plugin.log in its state dir).
pub fn append_plugin_log(plugin_id: &str, level: &str, message: &str) {
    if let Ok(dir) = registry::plugin_state_dir(plugin_id) {
        let line = format!(
            "{} [{}] {}\n",
            chrono::Utc::now().to_rfc3339(),
            level,
            message
        );
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("plugin.log"))
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }
}
