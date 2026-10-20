//! What StreamNook does for a member once a session knows who they are, on
//! every client: refresh their member row, record which client they use, and
//! collect what the server has waiting for them (today's seasonal and cake-day
//! accolades, the subscriber atmospheres they own, login-window and tenure
//! rewards).
//!
//! This used to run in the desktop page after its first sign-in check, so a
//! member who only used the iPhone never got a name, avatar or last-seen on
//! their row and never received any of it. It now runs here, once per member
//! per app session, the first time Rust confirms the signed-in Twitch user
//! (`TwitchService::get_user_info`, which every client calls at start and after
//! signing in). Seasonal windows open while a member stays signed in for days,
//! so it runs on every launch, not only on a fresh sign-in, as before.
//!
//! Every step is fire and forget and independent: one failing never stops the
//! others, and none can fail a sign-in.

use std::collections::HashSet;
use std::sync::Mutex;

use log::{debug, warn};
use once_cell::sync::Lazy;
use serde_json::{json, Value};

use crate::models::user::UserInfo;
use crate::services::member_rewards::{self as rewards, Gate};
use crate::services::streamnook_backend as backend;

const SYNC_PATH: &str = "/api/v1/user/sync";

/// Members already handled this app session.
static DONE: Lazy<Mutex<HashSet<String>>> = Lazy::new(Default::default);

/// The running client as the member row and `record_client` name it.
fn platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "desktop"
    }
}

/// The name shown for the member: their display name, or their login when Twitch gave none.
fn shown_name(user: &UserInfo) -> &str {
    if user.display_name.is_empty() {
        &user.login
    } else {
        &user.display_name
    }
}

/// The member row's fields, for the API sync.
fn sync_body(user: &UserInfo, app_version: Option<&str>) -> Value {
    json!({
        "username": user.login,
        "display_name": shown_name(user),
        "avatar_url": user.profile_image_url,
        "app_version": app_version,
        "platform": platform(),
    })
}

/// The member row for the legacy direct write.
fn legacy_row(user: &UserInfo, app_version: Option<&str>) -> Value {
    let mut row = json!({
        "id": user.id,
        "username": user.login,
        "display_name": shown_name(user),
        "avatar_url": user.profile_image_url,
        "last_seen": chrono::Utc::now().to_rfc3339(),
    });
    if let (Some(v), Some(map)) = (app_version, row.as_object_mut()) {
        map.insert("app_version".into(), Value::from(v));
    }
    row
}

/// The signed-in Twitch user is known. Runs the sign-in work for them once
/// this app session.
pub fn signed_in(user: &UserInfo) {
    if user.id.is_empty() || !DONE.lock().unwrap_or_else(|e| e.into_inner()).insert(user.id.clone()) {
        return;
    }
    let user = user.clone();
    tauri::async_runtime::spawn(async move { run(user).await });
}

async fn run(user: UserInfo) {
    let app_version = crate::services::providers::app_handle()
        .map(|app| crate::services::client_identity::current(&app).app_version);
    sync_member(&user, app_version.as_deref()).await;
    if !backend::configured() {
        return;
    }
    let id = user.id.as_str();
    match backend::rpc("claim_login_accolades", json!({ "p_twitch_user_id": id })).await {
        Ok(v) => debug!("[SignIn] accolades: {v}"),
        Err(e) => warn!("[SignIn] claim_login_accolades failed: {e}"),
    }
    // Server-gated on an active subscription; not deployed yet answers 404.
    if let Err(e) = backend::rpc("grant_atmosphere_ownership", json!({ "p_twitch_user_id": id })).await {
        if !e.contains("PGRST202") && !e.contains("42883") {
            warn!("[SignIn] grant_atmosphere_ownership failed: {e}");
        }
    }
    rewards::claim(id, Gate::Event, json!({})).await;
    rewards::claim(id, Gate::Milestone, json!({})).await;
}

/// The member row: through the API when the server switch says so (the server
/// takes the member from the token), else the legacy direct upsert and the
/// client record. A transport failure on the API falls back too, as the page
/// did, so a Cloudflare blip cannot stop the row being refreshed.
async fn sync_member(user: &UserInfo, app_version: Option<&str>) {
    if crate::services::client_config::get().await.write_via_api {
        match crate::commands::streamnook_api::post_json(SYNC_PATH, &sync_body(user, app_version), None).await {
            Ok(res) => {
                if !res.ok {
                    warn!("[SignIn] {SYNC_PATH} -> {} {}", res.status, res.body.chars().take(200).collect::<String>());
                }
                return;
            }
            Err(e) => warn!("[SignIn] {SYNC_PATH} unavailable, writing directly: {e}"),
        }
    }
    if !backend::configured() {
        return;
    }
    if let Err(e) = backend::upsert("users", "id", legacy_row(user, app_version)).await {
        warn!("[SignIn] users upsert failed: {e}");
    }
    if let Err(e) = backend::rpc("record_client", json!({ "p_user_id": user.id, "p_platform": platform() })).await {
        debug!("[SignIn] record_client failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(display: &str) -> UserInfo {
        UserInfo {
            id: "42".into(),
            login: "winters27".into(),
            display_name: display.into(),
            email: None,
            profile_image_url: Some("https://x/a.png".into()),
            broadcaster_type: None,
        }
    }

    #[test]
    fn the_sync_names_the_member_and_the_client() {
        let body = sync_body(&user("Winters27"), Some("8.9.0"));
        assert_eq!(body["username"], "winters27");
        assert_eq!(body["display_name"], "Winters27");
        assert_eq!(body["avatar_url"], "https://x/a.png");
        assert_eq!(body["app_version"], "8.9.0");
        assert_eq!(body["platform"], platform());
    }

    #[test]
    fn a_blank_display_name_falls_back_to_the_login() {
        assert_eq!(sync_body(&user(""), None)["display_name"], "winters27");
        let row = legacy_row(&user(""), None);
        assert_eq!(row["display_name"], "winters27");
        assert!(row.get("app_version").is_none());
        assert_eq!(legacy_row(&user("W"), Some("1.0"))["app_version"], "1.0");
    }

    #[test]
    fn runs_once_per_member_per_session() {
        let first = DONE.lock().unwrap().insert("once-test".into());
        let again = DONE.lock().unwrap().insert("once-test".into());
        assert!(first && !again);
    }
}
