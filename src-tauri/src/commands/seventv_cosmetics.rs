use crate::services::account_store::AccountStore;
use crate::services::seventv_auth_service::{
    SevenTVAuthService, SevenTVAuthStatus, SevenTVCosmeticsService,
};
use log::debug;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

#[derive(Debug, Serialize, Deserialize)]
pub struct SevenTVCosmeticsResult {
    pub success: bool,
    pub message: String,
}

/// The primary account's 7TV lives in the shared single-slot store (the one the
/// main Profile tab manages); linked secondaries use per-account storage. The
/// per-account commands route through this so the editor and the Profile tab
/// read ONE source for the main and never disagree about connection status.
fn is_primary_account(account_id: &str) -> bool {
    AccountStore::primary().map(|p| p.user_id).as_deref() == Some(account_id)
}

/// A persistent, isolated WebView profile directory for a linked account's 7TV
/// session. Keeping it on disk (vs incognito) means the account stays signed in
/// after the first login, so reconnects are frictionless and the token can be
/// refreshed silently by reloading the login page in this same profile.
fn seventv_profile_dir(account_id: &str) -> Result<std::path::PathBuf, String> {
    let mut path =
        crate::services::twitch_service::get_app_data_dir().map_err(|e| e.to_string())?;
    path.push("seventv_profiles");
    path.push(account_id);
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Decode the 7TV user id (`sub`) from the captured JWT, best-effort.
fn seventv_user_id_from_jwt(token: &str) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return String::new();
    }
    let payload_bytes = match URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(b) => b,
        Err(_) => return String::new(),
    };
    match serde_json::from_slice::<serde_json::Value>(&payload_bytes) {
        Ok(payload) => payload
            .get("sub")
            .or_else(|| payload.get("user_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        Err(_) => String::new(),
    }
}

/// Shared init-script that surfaces the 7TV token via an `about:blank#7TV_TOKEN=`
/// navigation the Rust side polls for.
const SEVENTV_TOKEN_CAPTURE_SCRIPT: &str = r#"
    (function() {
        let tokenFound = false;
        function checkForToken() {
            if (tokenFound) return;
            const token = localStorage.getItem('7tv-token');
            if (token && token.length > 50) {
                tokenFound = true;
                window.location.href = 'about:blank#7TV_TOKEN=' + encodeURIComponent(token);
                return;
            }
            setTimeout(checkForToken, 500);
        }
        setTimeout(checkForToken, 800);
        setInterval(() => { if (!tokenFound) checkForToken(); }, 1000);
    })();
"#;

// ============================================================================
// 7TV Auth Commands
// ============================================================================

/// Get 7TV authentication status
#[tauri::command]
pub async fn get_seventv_auth_status() -> SevenTVAuthStatus {
    SevenTVAuthService::get_auth_status().await
}

/// Get the 7TV login URL for the user to authenticate
#[tauri::command]
pub fn get_seventv_login_url() -> String {
    SevenTVAuthService::get_login_url()
}

/// Open 7TV login in an in-app browser window with automatic token capture
// Desktop-only: opens a 7TV login webview window. Mobile keeps cosmetics
// rendering/apply; the popup-based login is a desktop-only mechanism.
#[cfg(desktop)]
#[tauri::command]
pub async fn open_seventv_login_window(app: AppHandle) -> Result<bool, String> {
    let window_label = format!("seventv-login-{}", chrono::Utc::now().timestamp_millis());
    let login_url = SevenTVAuthService::get_login_url();

    debug!("[7TV] Opening login window: {}", login_url);

    // Simple JavaScript: find token, navigate to URL with token
    let token_capture_script = r#"
        (function() {
            console.log('[StreamNook 7TV] Token capture script loaded');
            let tokenFound = false;
            
            function checkForToken() {
                if (tokenFound) return;
                
                const token = localStorage.getItem('7tv-token');
                
                if (token && token.length > 50) {
                    tokenFound = true;
                    console.log('[StreamNook 7TV] Token found! Length:', token.length);
                    
                    // Show success overlay
                    const overlay = document.createElement('div');
                    overlay.style.cssText = 'position:fixed;top:0;left:0;right:0;bottom:0;background:rgba(0,0,0,0.9);z-index:999999;display:flex;align-items:center;justify-content:center;color:white;font-family:sans-serif;';
                    overlay.innerHTML = '<div style="text-align:center;"><div style="font-size:48px;margin-bottom:20px;">✓</div><h1 style="color:#29b6f6;">7TV Connected!</h1><p>This window will close automatically...</p></div>';
                    document.body.appendChild(overlay);
                    
                    // Navigate to URL with token - Rust will read this
                    setTimeout(() => {
                        window.location.href = 'about:blank#7TV_TOKEN=' + encodeURIComponent(token);
                    }, 1000);
                    return;
                }
                
                setTimeout(checkForToken, 500);
            }
            
            setTimeout(checkForToken, 1000);
            
            // Check on URL changes too
            setInterval(() => {
                if (!tokenFound) checkForToken();
            }, 1000);
        })();
    "#;

    let _window = WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::External(
            login_url
                .parse()
                .map_err(|e| format!("Invalid URL: {}", e))?,
        ),
    )
    .title("Connect 7TV Account")
    .inner_size(900.0, 700.0)
    .center()
    .visible(true)
    .decorations(true)
    .resizable(true)
    .initialization_script(token_capture_script)
    .build()
    .map_err(|e| format!("Failed to create window: {}", e))?;

    // Poll the window URL for the token
    let app_handle = app.clone();
    let label_clone = window_label.clone();

    tauri::async_runtime::spawn(async move {
        // Poll for up to 5 minutes
        for poll_count in 0..600 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            if let Some(win) = app_handle.get_webview_window(&label_clone) {
                // Check the current URL
                if let Ok(url) = win.url() {
                    let url_str = url.to_string();

                    // Log occasionally
                    if poll_count % 20 == 0 {
                        debug!(
                            "[7TV] Poll #{}: URL = {}",
                            poll_count,
                            &url_str[..url_str.len().min(50)]
                        );
                    }

                    // Check if URL contains our token marker
                    if url_str.contains("#7TV_TOKEN=") {
                        if let Some(token_part) = url_str.split("#7TV_TOKEN=").nth(1) {
                            if let Ok(token) = urlencoding::decode(token_part) {
                                debug!("[7TV] Token captured! Length: {}", token.len());

                                // Decode JWT to get user_id
                                let mut user_id = String::new();
                                if token.contains('.') {
                                    let parts: Vec<&str> = token.split('.').collect();
                                    if parts.len() == 3 {
                                        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
                                        use base64::Engine;
                                        if let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(parts[1])
                                        {
                                            if let Ok(payload_str) =
                                                String::from_utf8(payload_bytes)
                                            {
                                                if let Ok(payload) =
                                                    serde_json::from_str::<serde_json::Value>(
                                                        &payload_str,
                                                    )
                                                {
                                                    user_id = payload
                                                        .get("sub")
                                                        .or_else(|| payload.get("user_id"))
                                                        .and_then(|v| v.as_str())
                                                        .unwrap_or("")
                                                        .to_string();
                                                    debug!("[7TV] User ID from JWT: {}", user_id);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Store the token
                                match SevenTVAuthService::store_token(
                                    token.to_string(),
                                    user_id,
                                    String::new(),
                                )
                                .await
                                {
                                    Ok(_) => {
                                        debug!("[7TV] Token stored successfully!");
                                        let _ = app_handle.emit("seventv-connected", true);
                                    }
                                    Err(e) => debug!("[7TV] Failed to store token: {}", e),
                                }

                                // Close window
                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                let _ = win.close();
                                return;
                            }
                        }
                    }
                }
            } else {
                debug!("[7TV] Window closed by user");
                return;
            }
        }

        debug!("[7TV] Token capture timed out");
    });

    Ok(true)
}

/// Receive captured 7TV token from the browser window
#[tauri::command]
pub async fn receive_seventv_token(
    app: AppHandle,
    access_token: String,
    user_id: String,
    twitch_id: String,
) -> Result<bool, String> {
    SevenTVAuthService::store_token(access_token, user_id, twitch_id)
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("seventv-connected", true);
    Ok(true)
}

/// Store a 7TV token
#[tauri::command]
pub async fn store_seventv_token(
    access_token: String,
    user_id: String,
    twitch_id: String,
) -> Result<bool, String> {
    SevenTVAuthService::store_token(access_token, user_id, twitch_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Validate the current 7TV token
#[tauri::command]
pub async fn validate_seventv_token() -> bool {
    SevenTVAuthService::validate_token().await.unwrap_or(false)
}

/// Logout from 7TV
#[tauri::command]
pub async fn logout_seventv() -> Result<bool, String> {
    SevenTVAuthService::logout()
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Authenticated 7TV GraphQL passthrough for both reads and mutations. Attaches
/// the stored Bearer token (the primary's, or a linked account's when
/// `account_id` names a connected secondary) and returns the full JSON body
/// (data plus any non-auth errors) so the caller can surface things like a full
/// set or a name conflict. An expired/rejected token is cleared and surfaced as
/// SESSION_EXPIRED. The unauthenticated `seventv_graphql` stays for public reads
/// (e.g. the emote directory search).
#[tauri::command]
pub async fn seventv_graphql_authed(
    query: String,
    variables: Option<serde_json::Value>,
    operation_name: Option<String>,
    account_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let (token, cleanup) = match account_id.as_deref() {
        Some(id) if !is_primary_account(id) => (
            SevenTVAuthService::get_token_for(id)
                .await
                .map_err(|e| e.to_string())?,
            Some(id.to_string()),
        ),
        _ => (
            SevenTVAuthService::get_token()
                .await
                .map_err(|e| e.to_string())?,
            None,
        ),
    };

    let mut body = serde_json::Map::new();
    body.insert("query".into(), serde_json::Value::String(query));
    body.insert(
        "variables".into(),
        variables.unwrap_or_else(|| serde_json::Value::Object(Default::default())),
    );
    if let Some(op) = operation_name {
        body.insert("operationName".into(), serde_json::Value::String(op));
    }

    SevenTVCosmeticsService::post_authed(
        &token,
        cleanup.as_deref(),
        serde_json::Value::Object(body),
    )
    .await
    .map_err(|e| e.to_string())
}

// ============================================================================
// 7TV Cosmetics Commands
// ============================================================================

/// Set the user's active 7TV paint
#[tauri::command]
pub async fn set_seventv_paint(
    user_id: String,
    paint_id: Option<String>,
) -> Result<SevenTVCosmeticsResult, String> {
    match SevenTVCosmeticsService::set_active_paint(&user_id, paint_id.as_deref()).await {
        Ok(_) => Ok(SevenTVCosmeticsResult {
            success: true,
            message: match &paint_id {
                Some(id) => format!("Paint changed to {}", id),
                None => "Paint unequipped".to_string(),
            },
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Set the user's active 7TV badge
#[tauri::command]
pub async fn set_seventv_badge(
    user_id: String,
    badge_id: Option<String>,
) -> Result<SevenTVCosmeticsResult, String> {
    match SevenTVCosmeticsService::set_active_badge(&user_id, badge_id.as_deref()).await {
        Ok(_) => Ok(SevenTVCosmeticsResult {
            success: true,
            message: match &badge_id {
                Some(id) => format!("Badge changed to {}", id),
                None => "Badge unequipped".to_string(),
            },
        }),
        Err(e) => Err(e.to_string()),
    }
}

// ============================================================================
// Per-account 7TV (for linked secondary accounts)
// ============================================================================

/// 7TV auth status for a specific account. The primary reads the shared single
/// slot (same as the Profile tab); secondaries read their per-account store.
#[tauri::command]
pub async fn get_seventv_auth_status_for(account_id: String) -> SevenTVAuthStatus {
    if is_primary_account(&account_id) {
        SevenTVAuthService::get_auth_status().await
    } else {
        SevenTVAuthService::get_auth_status_for(&account_id).await
    }
}

/// Authoritatively verify an account's 7TV token against 7TV (network check).
/// Returns true only if 7TV accepts the token; a rejected token is cleared so
/// the next status read reflects reality.
#[tauri::command]
pub async fn validate_seventv_token_for(account_id: String) -> bool {
    if is_primary_account(&account_id) {
        SevenTVAuthService::validate_token().await.unwrap_or(false)
    } else {
        SevenTVAuthService::validate_token_for(&account_id)
            .await
            .unwrap_or(false)
    }
}

/// Disconnect an account's 7TV.
#[tauri::command]
pub async fn logout_seventv_for(account_id: String) -> Result<bool, String> {
    if is_primary_account(&account_id) {
        SevenTVAuthService::logout()
            .await
            .map_err(|e| e.to_string())?;
    } else {
        SevenTVAuthService::logout_for(&account_id)
            .await
            .map_err(|e| e.to_string())?;
        // Clear the persisted browser profile too, so disconnect is a true sign
        // out — otherwise lingering cookies would let a silent refresh re-mint a
        // token and quietly undo it.
        if let Ok(dir) = seventv_profile_dir(&account_id) {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    Ok(true)
}

/// Set an account's active 7TV paint (primary → single slot; secondary → its token).
#[tauri::command]
pub async fn set_seventv_paint_for(
    account_id: String,
    user_id: String,
    paint_id: Option<String>,
) -> Result<SevenTVCosmeticsResult, String> {
    let result = if is_primary_account(&account_id) {
        SevenTVCosmeticsService::set_active_paint(&user_id, paint_id.as_deref()).await
    } else {
        SevenTVCosmeticsService::set_active_paint_for(&account_id, &user_id, paint_id.as_deref())
            .await
    };
    match result {
        Ok(_) => Ok(SevenTVCosmeticsResult {
            success: true,
            message: match &paint_id {
                Some(id) => format!("Paint changed to {}", id),
                None => "Paint unequipped".to_string(),
            },
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Set an account's active 7TV badge (primary → single slot; secondary → its token).
#[tauri::command]
pub async fn set_seventv_badge_for(
    account_id: String,
    user_id: String,
    badge_id: Option<String>,
) -> Result<SevenTVCosmeticsResult, String> {
    let result = if is_primary_account(&account_id) {
        SevenTVCosmeticsService::set_active_badge(&user_id, badge_id.as_deref()).await
    } else {
        SevenTVCosmeticsService::set_active_badge_for(&account_id, &user_id, badge_id.as_deref())
            .await
    };
    match result {
        Ok(_) => Ok(SevenTVCosmeticsResult {
            success: true,
            message: match &badge_id {
                Some(id) => format!("Badge changed to {}", id),
                None => "Badge unequipped".to_string(),
            },
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Connect a linked account's 7TV via an INCOGNITO login window. Because the
/// window has its own isolated session (no shared twitch.tv cookies), the user
/// signs into Twitch as the alt there without disturbing the main's session, and
/// 7tv.app authenticates as the alt. The captured token is stored under the
/// given Twitch account id. `account_id` is the alt's Twitch user id.
#[cfg(desktop)]
#[tauri::command]
pub async fn open_seventv_login_window_for_account(
    app: AppHandle,
    account_id: String,
) -> Result<bool, String> {
    // The primary connects through its existing shared session (it IS the main
    // login), the same path the Profile tab uses, so the single slot stays the
    // one source of truth for the main.
    if is_primary_account(&account_id) {
        return open_seventv_login_window(app).await;
    }

    let window_label = format!("seventv-login-{}", chrono::Utc::now().timestamp_millis());
    let login_url = SevenTVAuthService::get_login_url();

    debug!(
        "[7TV] Opening incognito login window for account {}: {}",
        account_id, login_url
    );

    // Same token-capture script as the primary flow.
    let token_capture_script = r#"
        (function() {
            let tokenFound = false;
            function checkForToken() {
                if (tokenFound) return;
                const token = localStorage.getItem('7tv-token');
                if (token && token.length > 50) {
                    tokenFound = true;
                    const overlay = document.createElement('div');
                    overlay.style.cssText = 'position:fixed;top:0;left:0;right:0;bottom:0;background:rgba(0,0,0,0.9);z-index:999999;display:flex;align-items:center;justify-content:center;color:white;font-family:sans-serif;';
                    overlay.innerHTML = '<div style="text-align:center;"><div style="font-size:48px;margin-bottom:20px;">✓</div><h1 style="color:#29b6f6;">7TV Connected!</h1><p>This window will close automatically...</p></div>';
                    document.body.appendChild(overlay);
                    setTimeout(() => {
                        window.location.href = 'about:blank#7TV_TOKEN=' + encodeURIComponent(token);
                    }, 1000);
                    return;
                }
                setTimeout(checkForToken, 500);
            }
            setTimeout(checkForToken, 1000);
            setInterval(() => { if (!tokenFound) checkForToken(); }, 1000);
        })();
    "#;

    let _window = WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::External(
            login_url
                .parse()
                .map_err(|e| format!("Invalid URL: {}", e))?,
        ),
    )
    .title("Connect 7TV for this account")
    .inner_size(900.0, 700.0)
    .center()
    .visible(true)
    .decorations(true)
    .resizable(true)
    .data_directory(seventv_profile_dir(&account_id)?)
    .initialization_script(token_capture_script)
    .build()
    .map_err(|e| format!("Failed to create window: {}", e))?;

    let app_handle = app.clone();
    let label_clone = window_label.clone();
    let target_twitch_id = account_id.clone();

    tauri::async_runtime::spawn(async move {
        for poll_count in 0..600 {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            if let Some(win) = app_handle.get_webview_window(&label_clone) {
                if let Ok(url) = win.url() {
                    let url_str = url.to_string();
                    if poll_count % 20 == 0 {
                        debug!("[7TV] (account {}) poll #{}", target_twitch_id, poll_count);
                    }
                    if url_str.contains("#7TV_TOKEN=") {
                        if let Some(token_part) = url_str.split("#7TV_TOKEN=").nth(1) {
                            if let Ok(token) = urlencoding::decode(token_part) {
                                // Extract the 7TV user id from the JWT.
                                let mut seventv_user_id = String::new();
                                if token.contains('.') {
                                    let parts: Vec<&str> = token.split('.').collect();
                                    if parts.len() == 3 {
                                        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
                                        use base64::Engine;
                                        if let Ok(payload_bytes) = URL_SAFE_NO_PAD.decode(parts[1])
                                        {
                                            if let Ok(payload) =
                                                serde_json::from_slice::<serde_json::Value>(
                                                    &payload_bytes,
                                                )
                                            {
                                                seventv_user_id = payload
                                                    .get("sub")
                                                    .or_else(|| payload.get("user_id"))
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                            }
                                        }
                                    }
                                }

                                match SevenTVAuthService::store_token_for(
                                    &target_twitch_id,
                                    token.to_string(),
                                    seventv_user_id,
                                )
                                .await
                                {
                                    Ok(_) => {
                                        let _ = app_handle
                                            .emit("seventv-connected-account", &target_twitch_id);
                                    }
                                    Err(e) => debug!("[7TV] store_token_for failed: {}", e),
                                }

                                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                                let _ = win.close();
                                return;
                            }
                        }
                    }
                }
            } else {
                debug!("[7TV] account login window closed by user");
                return;
            }
        }
        debug!("[7TV] account token capture timed out");
    });

    Ok(true)
}

/// Silently refresh an account's 7TV token by reloading the login page in a
/// HIDDEN window using the account's persisted profile (or, for the primary, the
/// shared main session). If the underlying session is still valid the page
/// re-mints a token with no interaction and we recapture it. Returns true on
/// success; on timeout the caller falls back to a visible reconnect. Best-effort.
#[cfg(desktop)]
#[tauri::command]
pub async fn refresh_seventv_token_for_account(
    app: AppHandle,
    account_id: String,
) -> Result<bool, String> {
    let primary = is_primary_account(&account_id);
    let window_label = format!("seventv-refresh-{}", chrono::Utc::now().timestamp_millis());
    let login_url = SevenTVAuthService::get_login_url();

    let mut builder = WebviewWindowBuilder::new(
        &app,
        &window_label,
        WebviewUrl::External(
            login_url
                .parse()
                .map_err(|e| format!("Invalid URL: {}", e))?,
        ),
    )
    .title("Refreshing 7TV")
    .inner_size(900.0, 700.0)
    .visible(false)
    .initialization_script(SEVENTV_TOKEN_CAPTURE_SCRIPT);

    // Secondaries reload their own persisted profile; the primary uses the shared
    // main session (no separate data dir), same as the Profile tab.
    if !primary {
        builder = builder.data_directory(seventv_profile_dir(&account_id)?);
    }

    let win = builder
        .build()
        .map_err(|e| format!("Failed to create refresh window: {}", e))?;

    // Poll the hidden window for up to ~12s for a re-minted token.
    let mut captured = false;
    for _ in 0..24 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        match win.url() {
            Ok(url) => {
                let url_str = url.to_string();
                if let Some(token_part) = url_str.split("#7TV_TOKEN=").nth(1) {
                    if let Ok(token) = urlencoding::decode(token_part) {
                        let seventv_user_id = seventv_user_id_from_jwt(&token);
                        let stored = if primary {
                            SevenTVAuthService::store_token(
                                token.to_string(),
                                seventv_user_id,
                                String::new(),
                            )
                            .await
                        } else {
                            SevenTVAuthService::store_token_for(
                                &account_id,
                                token.to_string(),
                                seventv_user_id,
                            )
                            .await
                        };
                        captured = stored.is_ok();
                        break;
                    }
                }
            }
            Err(_) => break, // window closed/gone
        }
    }

    let _ = win.close();
    Ok(captured)
}
