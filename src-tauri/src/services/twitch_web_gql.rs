//! Twitch's private GQL, the way the twitch.tv web client calls it.
//!
//! Features Twitch keeps out of its public API (chat GIFs, cheering, the bits
//! balance) go through `gql.twitch.tv` under the web client id. A call that
//! acts as the viewer carries the twitch.tv web session the app's own sign-in
//! creates (`TwitchAuthService`): Twitch refuses StreamNook's own login token
//! under the web client id ("The Authorization token is invalid"). A call
//! about public data goes without any token.

use std::time::Duration;

use crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID;
use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_service::gql_device_id;

const GQL_URL: &str = "https://gql.twitch.tv/gql";

/// Bound on reading the web session. On a phone it waits on the UI thread, and
/// a backgrounded webview must not hang a picker or a send.
const TOKEN_BUDGET: Duration = Duration::from_secs(30);

/// The twitch.tv session token, or "not signed in to Twitch". Never prompts:
/// the session comes from the app's sign-in.
pub async fn web_session_token(auth: &TwitchAuthService) -> Result<String, String> {
    match tokio::time::timeout(TOKEN_BUDGET, auth.get_token()).await {
        Ok(Ok(token)) => Ok(token),
        _ => Err("not signed in to Twitch".to_string()),
    }
}

/// The app's twitch.tv web session from anywhere in the core, or `None` when
/// the viewer is not signed in to Twitch (or the app has not started yet).
pub async fn app_web_session() -> Option<String> {
    use tauri::Manager;
    let app = crate::services::providers::app_handle()?;
    let auth = app.try_state::<crate::models::settings::AppState>().map(|s| s.twitch_auth.clone())?;
    web_session_token(&auth).await.ok().filter(|t| !t.is_empty())
}

/// POST one inline GQL operation. `token` is the web session for calls made as
/// the viewer, `None` for public data. GraphQL `errors` are logged under `tag`
/// and returned in the JSON for the caller to read, never swallowed.
pub async fn post(token: Option<&str>, body: serde_json::Value, tag: &str) -> Result<serde_json::Value, String> {
    post_with(token, body, tag, None).await
}

/// `post` carrying a Client-Integrity token, for the mutations that refuse a
/// call without one. The call goes out as the device and browser the token
/// was minted for (`services::twitch_integrity`).
pub async fn post_with(
    token: Option<&str>,
    body: serde_json::Value,
    tag: &str,
    integrity: Option<&crate::services::twitch_integrity::Integrity>,
) -> Result<serde_json::Value, String> {
    let mut req = crate::services::http::client().post(GQL_URL).header("Client-ID", TWITCH_WEB_CLIENT_ID);
    req = match integrity {
        Some(i) => req
            .header("X-Device-ID", &i.device_id)
            .header("Client-Integrity", &i.token)
            .header("User-Agent", &i.user_agent),
        None => req.header("X-Device-ID", gql_device_id()),
    };
    if let Some(token) = token {
        req = req.header("Authorization", format!("OAuth {}", token));
    }
    let resp = req.json(&body).send().await.map_err(|e| format!("GQL request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GQL answered {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().await.map_err(|e| format!("GQL response was not JSON: {e}"))?;
    if let Some(errors) = json.get("errors") {
        log::warn!("[{tag}] GQL errors: {errors}");
    }
    Ok(json)
}
