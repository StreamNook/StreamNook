//! Server-controlled client switches, read from the update manifest.
//!
//! Exists so the writes that moved from direct Supabase to the StreamNook API
//! can be turned back off WITHOUT shipping a build. A desktop release cannot be
//! recalled: if the new write path has a bug, is rate-limited, or an endpoint is
//! rolled back, every user on that build silently loses writes until they
//! install another one. Flipping one field in the manifest reverts them in
//! minutes. It rides on the update manifest rather than a new endpoint because
//! that file is already fetched, already edge-cached (~300 s), and already the
//! thing a release re-uploads.
//!
//! FAILS OPEN, deliberately. If the config cannot be fetched the answer is the
//! legacy behaviour (direct Supabase), because the alternative is that a
//! Cloudflare blip takes writes down as well.

use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MANIFEST_URL: &str = "https://streamnook.app/api/v1/update";
const TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct ClientConfig {
    /// Route privileged writes through the StreamNook API instead of Supabase.
    pub write_via_api: bool,
    /// Builds below this are asked to update; empty means no floor.
    pub min_supported_version: String,
}

/// Held across the fetch, so a burst of writes at login shares one request.
static CACHE: Mutex<Option<(Instant, ClientConfig)>> = Mutex::const_new(None);

/// Read the switches out of a manifest body.
///
/// `min_supported` is read outside the `client_config` block on purpose: it is
/// a TOP-LEVEL manifest field (the updater reads it there), and older clients
/// looked for `client_config.min_supported_version` only. Prefer the block when
/// present and fall back to the top-level field, so a manifest written by either
/// convention works.
pub fn parse(body: &serde_json::Value) -> ClientConfig {
    let top_level_floor = body
        .get("min_supported")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let Some(block) = body.get("client_config").filter(|v| v.is_object()) else {
        return ClientConfig { min_supported_version: top_level_floor, ..Default::default() };
    };
    ClientConfig {
        // Only an explicit `true` enables it. A malformed or missing value must
        // never silently switch every client onto the new path.
        write_via_api: block.get("write_via_api") == Some(&serde_json::Value::Bool(true)),
        min_supported_version: block
            .get("min_supported_version")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or(top_level_floor),
    }
}

async fn fetch() -> Result<ClientConfig, String> {
    let body: serde_json::Value = crate::services::http::client()
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse(&body))
}

/// The current switches, cached for five minutes. A failed fetch answers with
/// the legacy behaviour and is not cached, so the next ask retries.
pub async fn get() -> ClientConfig {
    let mut slot = CACHE.lock().await;
    if let Some((at, config)) = slot.as_ref() {
        if at.elapsed() < TTL {
            return config.clone();
        }
    }
    match fetch().await {
        Ok(config) => {
            *slot = Some((Instant::now(), config.clone()));
            config
        }
        Err(e) => {
            // `warn`: a config fetch that always failed must leave a trace, since
            // it silently keeps every client on the legacy write path.
            log::warn!("[ClientConfig] fetch failed, using legacy behaviour: {e}");
            ClientConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_explicit_true_turns_api_writes_on() {
        assert!(parse(&json!({ "client_config": { "write_via_api": true } })).write_via_api);
        assert!(!parse(&json!({ "client_config": { "write_via_api": "true" } })).write_via_api);
        assert!(!parse(&json!({ "client_config": {} })).write_via_api);
        assert!(!parse(&json!({})).write_via_api);
    }

    #[test]
    fn the_floor_reads_either_convention() {
        let top = parse(&json!({ "min_supported": "8.5.0" }));
        assert_eq!(top.min_supported_version, "8.5.0");
        let block = parse(&json!({
            "min_supported": "8.5.0",
            "client_config": { "min_supported_version": "8.6.0" }
        }));
        assert_eq!(block.min_supported_version, "8.6.0");
        let fallback = parse(&json!({ "min_supported": "8.5.0", "client_config": {} }));
        assert_eq!(fallback.min_supported_version, "8.5.0");
    }
}
