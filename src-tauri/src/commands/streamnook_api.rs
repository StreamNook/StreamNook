//! One authenticated POST to StreamNook's own API, for the whole frontend.
//!
//! Every privileged write (profile theme, equipped cosmetic, user sync) used to
//! go straight to Supabase from the webview under the publishable anon key. That
//! key is in the shipped bundle, and the tables it wrote were world-writable by
//! user id, so anyone could set another member's theme, badge, or profile row.
//! Those writes now go to streamnook.app, which derives the acting user from the
//! bearer token instead of believing the request body.
//!
//! ONE command rather than one per endpoint, deliberately. A Tauri command in
//! this app is three edits (the `fn`, `generate_handler!`, and the ACL manifest),
//! and a command that reaches the handler but not the allowlist is silently
//! DENIED at invoke with no error anywhere a human would look. That has shipped
//! real bugs here three times. Five endpoints would have been fifteen chances to
//! miss one; this is three, once.
//!
//! The token stays in Rust. The frontend names a PATH, never a URL, and the path
//! is checked against a hardcoded allowlist below, so a compromised webview
//! cannot aim an authenticated request carrying the user's Twitch token at an
//! arbitrary host. That matters because streamer-authored HTML renders in this
//! webview.

use crate::services::account_store::AccountStore;
use crate::services::twitch_service::TwitchService;
use serde::Serialize;

const API_BASE: &str = "https://streamnook.app";

/// Paths the frontend may POST to. Exact matches only: no prefixes, no
/// interpolation, nothing derived from caller input. Adding an entry here is a
/// deliberate act, which is the point.
const ALLOWED_PATHS: &[&str] = &[
    "/api/v1/user/sync",
    "/api/cosmetics/theme",
    "/api/cosmetics/equip",
];

#[derive(Debug, Serialize)]
pub struct ApiResponse {
    /// HTTP status, so the caller can distinguish "rejected" from "offline".
    pub status: u16,
    pub ok: bool,
    /// Raw body. Callers parse what they expect; this stays generic so adding an
    /// endpoint needs no new Rust type.
    pub body: String,
}

/// Paths the overlay builder may call with any of GET/POST/DELETE. Exact
/// path or `/api/overlays/<id>` (one segment, no slashes inside).
const OVERLAY_PREFIX: &str = "/api/overlays";

fn overlay_path_allowed(path: &str) -> bool {
    match path.strip_prefix(OVERLAY_PREFIX) {
        None => false,
        Some("") => true,
        Some(rest) => rest.starts_with('/')
            && rest.len() > 1
            && !rest[1..].contains('/')
            && !rest.contains("..")
            && !rest.contains('?')
            && !rest.contains('#'),
    }
}

/// Authenticated GET / POST / DELETE against streamnook.app for the overlay
/// builder, so its Twitch token never enters the page. `query` is appended
/// as-is (`all=1`), `body` is sent as JSON for POST.
#[tauri::command]
pub async fn streamnook_api_request(
    method: String,
    path: String,
    query: Option<String>,
    body: Option<serde_json::Value>,
) -> Result<ApiResponse, String> {
    if !overlay_path_allowed(&path) {
        return Err(format!("path_not_allowed: {}", path));
    }
    if let Some(q) = &query {
        if q.contains('/') || q.contains('?') || q.contains('#') {
            return Err("bad_query".to_string());
        }
    }
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("no_token: {}", e))?;
    let client = crate::services::http::client();
    let url = match query {
        Some(q) if !q.is_empty() => format!("{}{}?{}", API_BASE, path, q),
        _ => format!("{}{}", API_BASE, path),
    };
    let req = match method.to_ascii_uppercase().as_str() {
        "GET" => client.get(&url),
        "DELETE" => client.delete(&url),
        "POST" => client.post(&url).json(&body.unwrap_or(serde_json::Value::Null)),
        other => return Err(format!("method_not_allowed: {}", other)),
    };
    let resp = req
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| format!("network: {}", e))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok(ApiResponse {
        status: status.as_u16(),
        ok: status.is_success(),
        body: text,
    })
}

/// POST to an allowlisted StreamNook API path, authenticated as the current
/// account (or a named linked account).
///
/// Returns Err only for conditions the caller can act on differently from an
/// HTTP error: no token, a disallowed path, or the request never completing. A
/// 4xx/5xx comes back as Ok with `ok: false` so callers can inspect the body.
#[tauri::command]
pub async fn streamnook_api_post(
    path: String,
    body: serde_json::Value,
    account_id: Option<String>,
) -> Result<ApiResponse, String> {
    if !ALLOWED_PATHS.contains(&path.as_str()) {
        return Err(format!("path_not_allowed: {}", path));
    }

    // Authenticate as the chosen account when one is given (a linked secondary),
    // else the primary. The server upserts whichever account the bearer resolves
    // to, so each account can only ever write its own row.
    let token = match account_id {
        Some(id) => AccountStore::get_token_for(&id)
            .await
            .map_err(|e| format!("no_token_for_account:{}: {}", id, e))?,
        None => TwitchService::get_token()
            .await
            .map_err(|e| format!("no_token: {}", e))?,
    };

    let client = crate::services::http::client();
    let resp = client
        .post(format!("{}{}", API_BASE, path))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("network: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    Ok(ApiResponse {
        status: status.as_u16(),
        ok: status.is_success(),
        body: text,
    })
}

#[cfg(test)]
mod tests {
    use super::ALLOWED_PATHS;

    #[test]
    fn allowlist_holds_only_exact_streamnook_paths() {
        for p in ALLOWED_PATHS {
            assert!(p.starts_with('/'), "{p} must be a path, not a URL");
            assert!(!p.contains("://"), "{p} must not carry a scheme");
            assert!(!p.contains('*'), "{p} must be exact, not a pattern");
        }
    }

    #[test]
    fn overlay_paths_are_exact_or_one_id_deep() {
        use super::overlay_path_allowed;
        assert!(overlay_path_allowed("/api/overlays"));
        assert!(overlay_path_allowed("/api/overlays/abc123"));
        assert!(!overlay_path_allowed("/api/overlays/"));
        assert!(!overlay_path_allowed("/api/overlays/a/b"));
        assert!(!overlay_path_allowed("/api/overlays/../user"));
        assert!(!overlay_path_allowed("/api/overlaysX"));
        assert!(!overlay_path_allowed("/api/v1/user/sync"));
    }

    #[test]
    fn allowlist_rejects_traversal_and_foreign_hosts() {
        // The command compares with `contains`, so these can never match. Asserted
        // so a future refactor to prefix matching fails here instead of in the wild.
        for bad in [
            "https://evil.example/api",
            "/api/v1/user/sync/../../admin",
            "//evil.example/api/v1/user/sync",
            "/api/admin/membership",
        ] {
            assert!(!ALLOWED_PATHS.contains(&bad), "{bad} must not be allowed");
        }
    }
}
