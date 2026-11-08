use crate::services::account_store::AccountStore;
use crate::services::seventv_auth_service::{
    SevenTVAuthService, SevenTVAuthStatus, SevenTVCosmeticsService,
};
use log::debug;
use serde::{Deserialize, Serialize};
use crate::rt::{AppHandle, WebviewWindowBuilder};
use tauri::{Emitter, Manager, WebviewUrl};

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
    let base =
        crate::services::twitch_service::get_app_data_dir().map_err(|e| e.to_string())?;
    let path = crate::platform::webview_store::profile_dir(
        base,
        std::path::Path::new("seventv_profiles").join(account_id),
    );
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
                if let Some(url) = crate::platform::webview_url::current_url(&win).await {
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

/// Store a 7TV token. An empty `user_id` is read from the token itself (its
/// `sub` claim) and an empty `twitch_id` is the primary account, so a native
/// shell hands over only what the 7TV page wrote.
#[tauri::command]
pub async fn store_seventv_token(
    access_token: String,
    user_id: String,
    twitch_id: String,
) -> Result<bool, String> {
    let user_id = if user_id.is_empty() {
        SevenTVAuthService::token_user_id(&access_token).ok_or_else(|| "token captured but carried no user id".to_string())?
    } else {
        user_id
    };
    let twitch_id = if twitch_id.is_empty() {
        crate::services::account_store::AccountStore::primary().map(|a| a.user_id).unwrap_or_default()
    } else {
        twitch_id
    };
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
        Ok(_) => {
            own_pick_made(Some(paint_id.as_deref()), None).await;
            Ok(SevenTVCosmeticsResult {
                success: true,
                message: match &paint_id {
                    Some(id) => format!("Paint changed to {}", id),
                    None => "Paint unequipped".to_string(),
                },
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// Uploads a new 7TV profile picture for the primary account (`user_id` is the
/// 7TV user id; the image as base64: PNG, JPG, GIF, WebP or AVIF up to 7 MB).
/// 7TV processes it before it shows; answers the pending picture's id.
#[tauri::command]
pub async fn upload_seventv_profile_picture(user_id: String, data_base64: String) -> Result<String, String> {
    use base64::Engine;
    use crate::services::seventv_auth_service::{profile_picture_mime, SevenTVCosmeticsService};
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| "That image could not be read.".to_string())?;
    if bytes.len() > SevenTVCosmeticsService::PROFILE_PICTURE_MAX_BYTES {
        return Err("That image is larger than 7 MB.".to_string());
    }
    let mime = profile_picture_mime(&bytes).ok_or_else(|| "Use a PNG, JPG, GIF, WebP or AVIF image.".to_string())?;
    SevenTVCosmeticsService::upload_profile_picture(&user_id, bytes, mime).await.map_err(|e| e.to_string())
}

/// Set the user's active 7TV badge
#[tauri::command]
pub async fn set_seventv_badge(
    user_id: String,
    badge_id: Option<String>,
) -> Result<SevenTVCosmeticsResult, String> {
    match SevenTVCosmeticsService::set_active_badge(&user_id, badge_id.as_deref()).await {
        Ok(_) => {
            own_pick_made(None, Some(badge_id.as_deref())).await;
            Ok(SevenTVCosmeticsResult {
                success: true,
                message: match &badge_id {
                    Some(id) => format!("Badge changed to {}", id),
                    None => "Badge unequipped".to_string(),
                },
            })
        }
        Err(e) => Err(e.to_string()),
    }
}

/// After the primary account's paint or badge changed on 7TV: hold the pick over
/// 7TV's lagging reads, and repaint the account's rows already on a native
/// shell's screen.
async fn own_pick_made(paint: Option<Option<&str>>, badge: Option<Option<&str>>) {
    use crate::services::{chat_row_view, seventv_cosmetics_resolver as resolver};
    let Some(twitch_id) = crate::services::account_store::AccountStore::primary().map(|a| a.user_id) else {
        return;
    };
    resolver::note_own_pick(&twitch_id, paint, badge);
    if chat_row_view::enabled() {
        let worn = resolver::resolve(twitch_id.clone()).await;
        let view = chat_row_view::worn_view(&worn.data.paints, &worn.data.badges);
        let frame = chat_row_view::cosmetics_frame("", "twitch", &twitch_id, &view);
        crate::services::irc_service::post_frame_to_all(frame).await;
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
                if let Some(url) = crate::platform::webview_url::current_url(&win).await {
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
/// HIDDEN window. If the Twitch session behind it is alive, 7TV's OAuth round
/// trip completes on its own and the callback writes a new token. Returns true
/// only when a token NEWER than the stored one was captured; on false the
/// caller falls back to a visible reconnect. Best-effort.
#[cfg(desktop)]
#[tauri::command]
pub async fn refresh_seventv_token_for_account(
    app: AppHandle,
    account_id: String,
) -> Result<bool, String> {
    Ok(remint_seventv(&app, &account_id).await)
}

/// The capture script for a silent re-mint: it reports only a token whose JWT
/// `exp` is past `min_exp`. 7tv.app keeps the OLD token in localStorage until
/// its login callback replaces it, so the plain capture script grabbed that
/// old token the moment the page loaded and "refreshed" to the same expiry
/// (logged as a success every boot of the last week, then signed out).
fn remint_capture_script(min_exp: i64) -> String {
    format!(
        r#"
    (function() {{
        const MIN_EXP = {min_exp};
        function expOf(t) {{
            try {{
                const p = t.split('.')[1].replace(/-/g, '+').replace(/_/g, '/');
                return JSON.parse(atob(p + '==='.slice((p.length + 3) % 4))).exp || 0;
            }} catch (e) {{ return 0; }}
        }}
        let done = false;
        function check() {{
            if (done) return;
            const t = localStorage.getItem('7tv-token');
            if (t && t.length > 50 && expOf(t) > MIN_EXP) {{
                done = true;
                window.location.href = 'about:blank#7TV_TOKEN=' + encodeURIComponent(t);
            }}
        }}
        setInterval(check, 500);
    }})();
"#
    )
}

/// The stored token for an account: (exp, 7TV user id). The primary lives in
/// the single slot, linked accounts in their own files.
async fn stored_seventv_token(account_id: &str, primary: bool) -> Option<(i64, Option<String>)> {
    let token = if primary {
        SevenTVAuthService::get_token_info()
            .await
            .ok()?
            .access_token
    } else {
        SevenTVAuthService::get_token_for(account_id).await.ok()?
    };
    Some((
        SevenTVAuthService::token_expires_at(&token).unwrap_or(0),
        SevenTVAuthService::token_user_id(&token),
    ))
}

/// Whether a captured token may replace the stored one: strictly newer, and
/// the same 7TV account when the stored one names it (a profile signed into a
/// different Twitch account must never swap whose 7TV this is).
pub(crate) fn remint_accepts(captured: &str, min_exp: i64, stored_user: Option<&str>) -> bool {
    let newer = SevenTVAuthService::token_expires_at(captured).is_some_and(|exp| exp > min_exp);
    let same_user = match (stored_user, SevenTVAuthService::token_user_id(captured)) {
        (Some(had), Some(got)) => had == got,
        _ => true,
    };
    newer && same_user
}

/// One hidden-window attempt in one browser profile (`None` is the app's
/// default profile). Polls up to `secs` for a token `remint_accepts` takes.
#[cfg(desktop)]
async fn remint_in_profile(
    app: &AppHandle,
    data_dir: Option<std::path::PathBuf>,
    min_exp: i64,
    stored_user: Option<&str>,
    secs: u64,
) -> Option<String> {
    let label = format!("seventv-refresh-{}", chrono::Utc::now().timestamp_millis());
    let url = SevenTVAuthService::get_login_url().parse().ok()?;
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url))
        .title("Refreshing 7TV")
        .inner_size(900.0, 700.0)
        .visible(false)
        .initialization_script(&remint_capture_script(min_exp));
    if let Some(dir) = data_dir {
        builder = builder.data_directory(dir);
    }
    let win = match builder.build() {
        Ok(w) => w,
        Err(e) => {
            log::info!("[7TV] re-mint window could not open: {e}");
            return None;
        }
    };
    let mut got = None;
    for _ in 0..secs * 2 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let Some(url) = crate::platform::webview_url::current_url(&win).await else {
            continue;
        };
        let url = url.to_string();
        let Some(part) = url.split("#7TV_TOKEN=").nth(1) else {
            continue;
        };
        if let Ok(token) = urlencoding::decode(part) {
            if remint_accepts(&token, min_exp, stored_user) {
                got = Some(token.into_owned());
            } else {
                log::info!(
                    "[7TV] re-mint captured a token that is not newer or not this account; ignored"
                );
            }
        }
        break;
    }
    let _ = win.close();
    got
}

/// Re-mint an account's 7TV session without a window the user sees. Tries the
/// profile the account connected 7TV in first (the app's default profile for
/// the primary, its own 7TV profile for a linked account), then the account's
/// Twitch web profile, the one the app keeps signed into Twitch, so the OAuth
/// round trip completes even when the first profile's Twitch login has lapsed.
#[cfg(desktop)]
pub async fn remint_seventv(app: &AppHandle, account_id: &str) -> bool {
    let primary = is_primary_account(account_id);
    let Some((min_exp, stored_user)) = stored_seventv_token(account_id, primary).await else {
        // Never connected, or signed out on purpose: nothing to keep alive.
        return false;
    };
    // `None` is the app's default profile, where the primary connected 7TV.
    let mut profiles: Vec<Option<std::path::PathBuf>> = Vec::new();
    if primary {
        profiles.push(None);
    } else if let Ok(dir) = seventv_profile_dir(account_id) {
        profiles.push(Some(dir));
    }
    if let Ok(dir) = crate::services::twitch_service::twitch_web_profile_dir(account_id) {
        profiles.push(Some(dir));
    }
    for dir in profiles {
        let Some(token) = remint_in_profile(app, dir, min_exp, stored_user.as_deref(), 30).await
        else {
            continue;
        };
        let user = seventv_user_id_from_jwt(&token);
        let stored = if primary {
            SevenTVAuthService::store_token(token, user, String::new()).await
        } else {
            SevenTVAuthService::store_token_for(account_id, token, user).await
        };
        if stored.is_ok() {
            log::info!("[7TV] session re-minted silently");
            let _ = if primary {
                app.emit("seventv-connected", true)
            } else {
                app.emit("seventv-connected-account", account_id)
            };
            return true;
        }
    }
    log::info!("[7TV] silent re-mint did not complete");
    false
}

/// Start re-minting this far ahead of a token's expiry.
const REMINT_AHEAD_SECS: i64 = 7 * 24 * 60 * 60;

/// Whether a token expiring at `exp` is due for a re-mint at `now`.
pub(crate) fn remint_due(exp: i64, now: i64) -> bool {
    exp - now <= REMINT_AHEAD_SECS
}

/// Keeps every connected account's 7TV session alive while the app runs: a
/// first look shortly after launch, then every 6 hours, so an app left open
/// for weeks re-mints too (the old check ran once per launch, from the page).
#[cfg(desktop)]
pub fn spawn_seventv_keepalive(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        loop {
            let now = chrono::Utc::now().timestamp();
            for account in AccountStore::list() {
                let primary = is_primary_account(&account.user_id);
                let Some((exp, _)) = stored_seventv_token(&account.user_id, primary).await else {
                    continue;
                };
                if remint_due(exp, now) {
                    remint_seventv(&app, &account.user_id).await;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

#[cfg(test)]
mod remint_tests {
    use super::{remint_accepts, remint_due};
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    fn jwt(exp: i64, sub: &str) -> String {
        let payload = serde_json::json!({ "exp": exp, "sub": sub }).to_string();
        format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn the_old_token_still_in_storage_is_not_a_refresh() {
        // 7tv.app holds the old token until its callback runs; reading that
        // back must not count as a re-mint.
        assert!(!remint_accepts(&jwt(1_000, "u1"), 1_000, Some("u1")));
        assert!(!remint_accepts(&jwt(900, "u1"), 1_000, Some("u1")));
        assert!(remint_accepts(&jwt(1_001, "u1"), 1_000, Some("u1")));
    }

    #[test]
    fn a_profile_signed_into_someone_else_never_swaps_the_account() {
        assert!(!remint_accepts(&jwt(5_000, "u2"), 1_000, Some("u1")));
        assert!(remint_accepts(&jwt(5_000, "u2"), 1_000, None));
    }

    #[test]
    fn a_session_is_due_a_week_before_it_ends_and_after() {
        let day = 24 * 60 * 60;
        assert!(!remint_due(10 * day, 0));
        assert!(remint_due(7 * day, 0));
        assert!(remint_due(-day, 0));
    }
}
