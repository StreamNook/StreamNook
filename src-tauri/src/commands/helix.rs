//! Authenticated Helix reads made by Rust on the page's behalf.
//!
//! Until 2026-09-07 fifteen surfaces asked Rust for the app client id and the
//! user's OAuth token (`get_twitch_credentials`) and called
//! `api.twitch.tv/helix/...` from the WebView themselves. The token therefore
//! lived in page JavaScript, reachable by anything that ran there. Now the
//! page names the resource and the query; Rust attaches the credentials,
//! makes the request and hands back the JSON. The resource allowlist is the
//! exact set the app reads; anything else is refused before a token is even
//! loaded. The Twitch web login cookie is a different secret and is untouched
//! by this: it stays in the WebView2 profile and is only ever read by the
//! auth services.

use reqwest::header::AUTHORIZATION;

use crate::services::twitch_service::TwitchService;

const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");

/// Helix resources the page may read. Exact path segments after `/helix/`.
const ALLOWED_RESOURCES: &[&str] = &["users", "streams", "channels", "clips"];

fn query_is_plain(query: &str) -> bool {
    !query.is_empty()
        && query.len() <= 8192
        && !query.contains('/')
        && !query.contains('?')
        && !query.contains('#')
        && !query.contains("://")
}

/// GET `https://api.twitch.tv/helix/{resource}?{query}` with the app's
/// credentials. Returns the parsed JSON body on 2xx; a non-2xx status is an
/// error carrying the status code so callers can keep their old `resp.ok`
/// branches as `catch`.
#[tauri::command]
pub async fn helix_get(resource: String, query: String) -> Result<serde_json::Value, String> {
    if !ALLOWED_RESOURCES.contains(&resource.as_str()) {
        return Err(format!("resource_not_allowed: {resource}"));
    }
    if !query_is_plain(&query) {
        return Err("bad_query".to_string());
    }
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("no_token: {e}"))?;
    let client = crate::services::http::client().clone();
    let url = format!("https://api.twitch.tv/helix/{resource}?{query}");
    let resp = client
        .get(&url)
        .header("Client-Id", CLIENT_ID)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let body = serde_json::from_str::<serde_json::Value>(&text).ok();
    if !status.is_success() {
        return Err(format!(
            "helix_{}: {}",
            status.as_u16(),
            body.map(|b| b.to_string()).unwrap_or(text)
        ));
    }
    // A 2xx that is not JSON (a proxy page, a truncated body) is an error, not
    // a `null` the caller would then index into.
    body.ok_or_else(|| "helix_bad_body".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_is_exact_resources() {
        for r in ALLOWED_RESOURCES {
            assert!(!r.contains('/') && !r.contains('?'), "{r}");
        }
        assert!(!ALLOWED_RESOURCES.contains(&"users/follows"));
    }

    #[test]
    fn query_rejects_path_and_scheme_tricks() {
        assert!(query_is_plain("login=a&login=b"));
        assert!(query_is_plain("id=1&first=1"));
        assert!(!query_is_plain(""));
        assert!(!query_is_plain("login=a/../../other"));
        assert!(!query_is_plain("x=1?y=2"));
        assert!(!query_is_plain("x=1#frag"));
        assert!(!query_is_plain("x=https://evil"));
    }
}
