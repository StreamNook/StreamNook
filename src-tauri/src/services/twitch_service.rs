use crate::models::settings::AppState;
use crate::models::{
    stream::TwitchStream,
    user::{ChannelInfo, UserInfo},
};
use crate::services::cookie_jar_service::CookieJarService;
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use crate::services::secure_store::Entry;
use log::{debug, error, warn};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Emitter;

const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
const CLIENT_SECRET: &str = env!("TWITCH_APP_CLIENT_SECRET");
const TWITCH_GQL_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const KEYRING_SERVICE: &str = "streamnook_twitch_token";
const KEYRING_USERNAME: &str = "user"; // Standardized username
const REDIRECT_URI: &str = "http://localhost:3000/callback";
// Adding ANY scope here invalidates every stored token at once (see the
// missing-scopes branch in check_token_health, which calls AccountStore::
// reset_all). That makes a scope change a one-time, whole-user-base re-auth, so
// new scopes are batched deliberately rather than added one at a time.
//
// The 2026-07 batch added, in one event:
//   channel:manage:polls / channel:manage:predictions — create and resolve them
//   moderator:manage:blocked_terms — /blockterm and /unblockterm (read-only
//     moderator:read:blocked_terms was already held)
//   user:bot — the chat-bot badge on /bot sends
const SCOPES: &str = "user:read:follows user:read:email chat:read chat:edit channel:read:redemptions channel:manage:redemptions moderator:read:followers openid user:manage:whispers user:read:whispers user:read:emotes channel:read:hype_train moderator:read:blocked_terms moderator:manage:blocked_terms moderator:manage:chat_settings moderator:manage:unban_requests moderator:manage:banned_users moderator:manage:chat_messages moderator:read:warnings moderator:read:moderators moderator:read:vips moderator:read:chatters channel:manage:moderators channel:manage:vips channel:manage:polls channel:manage:predictions moderator:manage:suspicious_users user:manage:chat_color user:manage:blocked_users user:read:blocked_users moderator:manage:announcements moderator:manage:shoutouts channel:edit:commercial channel:manage:raids channel:manage:broadcast moderation:read user:write:chat user:bot clips:edit moderator:manage:automod moderator:read:suspicious_users";
const TOKEN_FILE_NAME: &str = ".twitch_token";

/// Normalize a GQL Language enum spelling ("FR", "ZH_HK") to the Helix form
/// ("fr", "zh-hk") so `TwitchStream.language` is uniform across API paths.
pub(crate) fn normalize_gql_language(value: &str) -> String {
    value.to_ascii_lowercase().replace('_', "-")
}

static GQL_DEVICE_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Stable per-install device id for first-party GQL calls (recommendations,
/// similar channels, channel panels). Twitch keys its anonymous recommendation
/// profile and rate-limit bucket off X-Device-Id, so it must survive restarts.
pub(crate) fn gql_device_id() -> String {
    GQL_DEVICE_ID
        .get_or_init(|| {
            let path = get_app_data_dir().map(|d| d.join(".gql_device_id")).ok();
            if let Some(p) = &path {
                if let Ok(s) = fs::read_to_string(p) {
                    let s = s.trim().to_string();
                    if (16..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
                    {
                        return s;
                    }
                }
            }
            let id = uuid::Uuid::new_v4().to_string().replace('-', "");
            if let Some(p) = &path {
                if let Some(dir) = p.parent() {
                    let _ = fs::create_dir_all(dir);
                }
                let _ = fs::write(p, &id);
            }
            id
        })
        .clone()
}

/// Get the app data directory (works consistently in dev and release)
pub(crate) fn get_app_data_dir() -> Result<PathBuf> {
    // Mobile: app-private sandbox dir (desktop keeps the dirs-based path below).
    if let Some(base) = crate::services::app_paths::mobile_base() {
        return Ok(base.join("StreamNook"));
    }
    // Try to use the standard config directory first
    if let Some(config_dir) = dirs::config_dir() {
        let app_dir = config_dir.join("StreamNook");
        return Ok(app_dir);
    }

    // Fallback to data directory
    if let Some(data_dir) = dirs::data_dir() {
        let app_dir = data_dir.join("StreamNook");
        return Ok(app_dir);
    }

    // Last resort: use current exe directory
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let app_dir = exe_dir.join("data");
            return Ok(app_dir);
        }
    }

    Err(anyhow::anyhow!("Could not determine app data directory"))
}

/// Per-account WebView2 user-data folder for Twitch *web* sessions
/// (`app_data_dir/twitch_web_profiles/<account_id>`). Child webviews that load
/// twitch.tv (the subscribe page, the device-code activation page) are built
/// with this as their `data_directory`, so each linked account keeps its own
/// isolated, persistent browser session. Switching the main account therefore
/// switches the web session too, and a re-login can never silently inherit a
/// different account's session. Mirrors the 7TV per-account profile pattern.
pub(crate) fn twitch_web_profile_dir(account_id: &str) -> Result<PathBuf> {
    let mut path = get_app_data_dir()?;
    path.push("twitch_web_profiles");
    path.push(account_id);
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// Staging WebView2 profile for a login whose account id isn't known yet. The
/// device-code web session has to be captured somewhere before Twitch tells us
/// who signed in; a single fixed folder is safe because only one login is ever
/// in flight at once. `adopt_pending_web_profile` moves this into the account's
/// own profile once the id is known, so the subscribe window (and every other
/// per-account twitch webview) sees the same session the user logged into.
pub(crate) fn twitch_pending_web_profile_dir() -> Result<PathBuf> {
    let mut path = get_app_data_dir()?;
    path.push("twitch_web_profiles");
    path.push("_pending");
    fs::create_dir_all(&path)?;
    Ok(path)
}

/// The WebView2 profile directory holding the ACTIVE account's twitch.tv web
/// session. This is the single source of truth for the signed-in web session:
/// the login window, the subscribe window, and the stream resolver's token read
/// all bind to it, so they can never disagree about which account is signed in.
/// When an account is linked it is that account's own profile (adopting a
/// freshly-staged first login if one is waiting); before any account is recorded
/// it is the staging profile the login window writes to.
pub(crate) fn active_twitch_web_profile_dir() -> Result<PathBuf> {
    if let Some(primary) = crate::services::account_store::AccountStore::primary() {
        adopt_pending_web_profile(&primary.user_id);
        twitch_web_profile_dir(&primary.user_id)
    } else {
        twitch_pending_web_profile_dir()
    }
}

/// True when a WebView2 profile folder holds a twitch.tv session (its cookie
/// store has been written at least once).
fn web_profile_has_session(dir: &PathBuf) -> bool {
    dir.join("EBWebView")
        .join("Default")
        .join("Network")
        .join("Cookies")
        .exists()
}

/// Adopt a freshly-staged login session into `account_id`'s own web profile.
///
/// First sign-in captures the web session in the `_pending` staging profile
/// (the account id isn't known until the device-code grant completes). Once the
/// account is recorded, the next per-account twitch window calls this to move
/// that staged session into the account's own profile, so the login window and
/// the subscribe window stop disagreeing about which profile holds the session.
///
/// No-ops unless there's a staged session to adopt and the target doesn't
/// already have its own. Best-effort: a failure just leaves the user to
/// re-login, it never breaks the window.
pub(crate) fn adopt_pending_web_profile(account_id: &str) {
    let base = match get_app_data_dir() {
        Ok(b) => b.join("twitch_web_profiles"),
        Err(_) => return,
    };
    let pending = base.join("_pending");
    let target = base.join(account_id);

    // Nothing staged -> nothing to adopt.
    if !web_profile_has_session(&pending) {
        return;
    }
    // Target already owns a session -> keep it, discard the stale stage.
    if web_profile_has_session(&target) {
        let _ = fs::remove_dir_all(&pending);
        return;
    }
    // Move the staged session into place. Clear the empty placeholder first so
    // the rename has a clean destination (Windows rename fails if it exists).
    if target.exists() {
        let _ = fs::remove_dir_all(&target);
    }
    if let Err(e) = fs::rename(&pending, &target) {
        // Rename can fail if the staged profile is still locked or lives on a
        // different volume; fall back to a recursive copy so the session isn't
        // stranded, then drop the stage.
        debug!("[twitch-web] adopt rename failed ({e}); copying instead");
        if let Err(e) = copy_dir_recursive(&pending, &target) {
            warn!("[twitch-web] could not adopt staged login session: {e}");
            return;
        }
        let _ = fs::remove_dir_all(&pending);
    }
}

/// Minimal recursive directory copy (std has no equivalent). Only used as the
/// rename fallback in `adopt_pending_web_profile`.
fn copy_dir_recursive(from: &PathBuf, to: &PathBuf) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Best-effort removal of an account's Twitch web profile (called when the
/// account is unlinked or fully signed out, so a removed account leaves no
/// lingering browser session behind).
pub(crate) fn delete_twitch_web_profile(account_id: &str) {
    if let Ok(mut base) = get_app_data_dir() {
        base.push("twitch_web_profiles");
        base.push(account_id);
        if base.exists() {
            let _ = fs::remove_dir_all(&base);
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct StorableToken {
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: i64, // Unix timestamp
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Token health status returned by verify_token_health
#[derive(Debug, Clone, Serialize)]
pub struct TokenHealthStatus {
    pub is_valid: bool,
    pub seconds_remaining: i64,
    pub hours_remaining: i64,
    pub minutes_remaining: i64,
    pub scopes: Vec<String>,
    pub user_id: Option<String>,
    pub login: Option<String>,
    pub needs_refresh: bool,
    pub error: Option<String>,
}

pub struct TwitchService;

impl TwitchService {
    fn get_token_file_path() -> Result<PathBuf> {
        let mut path = get_app_data_dir()?;

        // Create directory if it doesn't exist
        if !path.exists() {
            debug!("[TWITCH_SERVICE] Creating directory: {:?}", path);
            fs::create_dir_all(&path)?;
        }

        path.push(TOKEN_FILE_NAME);
        debug!("[TWITCH_SERVICE] Token file path: {:?}", path);
        Ok(path)
    }

    /// Check if stored credentials exist (for showing appropriate toast)
    pub async fn has_stored_credentials() -> bool {
        // Check file first
        if let Ok(path) = Self::get_token_file_path() {
            if path.exists() {
                debug!("[TWITCH_SERVICE] Found stored token file");
                return true;
            }
        }

        // Check cookies
        if let Ok(cookie_jar) = CookieJarService::new_main() {
            if cookie_jar.has_auth_token().await {
                debug!("[TWITCH_SERVICE] Found stored cookie token");
                return true;
            }
        }

        // Check keyring
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
            if entry.get_password().is_ok() {
                debug!("[TWITCH_SERVICE] Found stored keyring token");
                return true;
            }
        }

        false
    }

    fn store_token_to_file(token: &StorableToken) -> Result<()> {
        let path = Self::get_token_file_path()?;
        let token_json = serde_json::to_string(token)?;

        // Simple XOR encryption with a fixed key for basic obfuscation
        let key: Vec<u8> = "StreamNookTokenKey2024"
            .bytes()
            .cycle()
            .take(token_json.len())
            .collect();
        let encrypted: Vec<u8> = token_json
            .bytes()
            .zip(key.iter())
            .map(|(a, b)| a ^ b)
            .collect();

        fs::write(&path, encrypted)?;
        debug!("[STORAGE] Token saved to file: {:?}", path);
        Ok(())
    }

    fn load_token_from_file() -> Result<StorableToken> {
        let path = Self::get_token_file_path()?;

        if !path.exists() {
            return Err(anyhow::anyhow!("Token file does not exist"));
        }

        let encrypted = fs::read(&path)?;

        // Decrypt using the same XOR method
        let key: Vec<u8> = "StreamNookTokenKey2024"
            .bytes()
            .cycle()
            .take(encrypted.len())
            .collect();
        let decrypted: String = encrypted
            .iter()
            .zip(key.iter())
            .map(|(a, b)| (a ^ b) as char)
            .collect();

        let token: StorableToken = serde_json::from_str(&decrypted)?;
        Ok(token)
    }

    fn delete_token_file() -> Result<()> {
        let path = Self::get_token_file_path()?;
        if path.exists() {
            fs::remove_file(&path)?;
            debug!("[STORAGE] Token file deleted: {:?}", path);
        }
        Ok(())
    }

    // Cookie-based storage methods
    async fn store_token_to_cookies(token: &StorableToken) -> Result<()> {
        let cookie_jar = CookieJarService::new_main()?;
        // Store full token data including refresh token and expiration
        cookie_jar
            .set_full_token_data(&token.access_token, &token.refresh_token, token.expires_at)
            .await?;
        debug!("[STORAGE] Full token data saved to cookies (access, refresh, expires_at)");
        Ok(())
    }

    async fn load_token_from_cookies() -> Result<StorableToken> {
        let cookie_jar = CookieJarService::new_main()?;

        let access_token = cookie_jar
            .get_auth_token()
            .await
            .ok_or_else(|| anyhow::anyhow!("No auth token in cookies"))?;

        let refresh_token = cookie_jar.get_refresh_token().await.unwrap_or_default();

        let expires_at = cookie_jar.get_token_expires_at().await.unwrap_or(0);

        Ok(StorableToken {
            access_token,
            refresh_token,
            expires_at,
        })
    }

    /// Write a token to ALL primary-slot storage (file + cookies + keyring). This
    /// is the single definition of "occupying the primary slot", so login,
    /// refresh, and account-switch all persist it identically. `refresh_token`
    /// deliberately does NOT call this: keeping refresh a pure transform is what
    /// stops a SECONDARY account's refresh (via `AccountStore::get_token_for`)
    /// from silently clobbering the primary's token.
    pub(crate) async fn persist_primary_token(token: &StorableToken) -> Result<()> {
        Self::store_token_to_file(token)?;
        let _ = Self::store_token_to_cookies(token).await;
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
            if let Ok(json) = serde_json::to_string(token) {
                let _ = entry.set_password(&json);
            }
        }
        Ok(())
    }

    /// Read whatever token currently occupies the primary slot (file first, then
    /// cookies). The account registry uses this to capture the outgoing main's
    /// token before a switch so the old main can be preserved as a linked
    /// secondary instead of being lost.
    pub(crate) async fn load_primary_token() -> Result<StorableToken> {
        if let Ok(token) = Self::load_token_from_file() {
            return Ok(token);
        }
        Self::load_token_from_cookies().await
    }

    async fn delete_cookies() -> Result<()> {
        let cookie_jar = CookieJarService::new_main()?;
        cookie_jar.clear().await?;
        debug!("[STORAGE] Cookies deleted");
        Ok(())
    }

    // Device Code Flow - the main login method.
    //
    // Returns `(verification_uri, user_code)` from a SINGLE device flow and
    // spawns the poller against that same flow's device_code. The caller shows
    // this user_code and opens this verification_uri, so the code the user
    // authorizes is always the one the poller is waiting on. (A previous version
    // started a second, separate device flow just to fetch a code to display,
    // which could leave the poller waiting on a code the user never authorized,
    // hanging the login and leaving the app unauthenticated.)
    pub async fn login(
        _state: &AppState,
        app_handle: tauri::AppHandle,
    ) -> Result<(String, String)> {
        let client = crate::services::http::client().clone();

        // Start device flow
        let device_response = Self::start_device_flow(&client).await?;
        let user_code = device_response.user_code.clone();

        debug!(
            "Device code flow started. User code: {}",
            device_response.user_code
        );

        // Clone values for the spawned task
        let device_code = device_response.device_code.clone();
        let interval = device_response.interval;
        let expires_in = device_response.expires_in;
        let verification_uri = device_response.verification_uri.clone();

        // Spawn a task to poll for token
        tokio::task::spawn(async move {
            debug!("[LOGIN] Starting token polling task...");
            let result = Self::poll_for_token(&client, &device_code, interval, expires_in).await;

            match result {
                Ok(token_response) => {
                    debug!("[LOGIN] Token received from Twitch!");
                    // Dismiss the in-app login overlay the instant we have the
                    // token, so the loading screen behind it shows without waiting
                    // on a frontend round-trip. The frontend dismisses it too as a
                    // backup. (Desktop-only overlay; no-op on mobile device-code login.)
                    #[cfg(desktop)]
                    crate::commands::twitch::dismiss_login_overlay(&app_handle, "twitch-login");
                    debug!(
                        "[LOGIN] Access token (first 10 chars): {}...",
                        &token_response.access_token[..10.min(token_response.access_token.len())]
                    );
                    debug!(
                        "[LOGIN] Refresh token present: {}",
                        token_response.refresh_token.is_some()
                    );
                    debug!(
                        "[LOGIN] Expires in: {} seconds",
                        token_response.expires_in.unwrap_or(0)
                    );

                    // Store the token
                    let expires_at = chrono::Utc::now()
                        + chrono::Duration::seconds(
                            token_response.expires_in.unwrap_or(3600) as i64
                        );

                    let storable_token = StorableToken {
                        access_token: token_response.access_token.clone(),
                        refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
                        expires_at: expires_at.timestamp(),
                    };

                    // Store token to both file and cookies for persistence
                    debug!("[LOGIN] Storing token to file and cookies...");

                    // Store to file (backward compatibility)
                    let file_result = Self::store_token_to_file(&storable_token);

                    // Store to cookies (new persistent storage)
                    let cookie_result = Self::store_token_to_cookies(&storable_token).await;

                    // The web-session reads (stream resolver, follow/unfollow,
                    // entitlement checks) gate on a logged_out flag that
                    // twitch_logout sets. A fresh login has to clear it and drop
                    // the stale cache, otherwise the next get_token() short-
                    // circuits to "not logged in" and every web-session feature
                    // stays dead until restart. This poller is the only place the
                    // primary login actually completes, so it owns the reset.
                    {
                        use tauri::Manager;
                        app_handle
                            .state::<crate::models::settings::AppState>()
                            .twitch_auth
                            .on_account_changed()
                            .await;
                    }

                    match (file_result, cookie_result) {
                        (Ok(_), Ok(_)) => {
                            debug!("[LOGIN] Token stored successfully to file and cookies!");

                            // Try to also store in keyring as backup (but don't fail if it doesn't work)
                            if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
                                if let Ok(token_json) = serde_json::to_string(&storable_token) {
                                    let _ = entry.set_password(&token_json);
                                    debug!("[LOGIN] Also stored token in keyring as backup");
                                }
                            }

                            // Emit success event
                            debug!("[LOGIN] Emitting twitch-login-complete event...");
                            if let Err(e) = app_handle.emit("twitch-login-complete", ()) {
                                error!("[LOGIN] Failed to emit login-complete event: {}", e);
                            } else {
                                debug!("[LOGIN] Event emitted successfully");
                            }
                        }
                        (Ok(_), Err(e)) => {
                            error!("[LOGIN] Token saved to file but cookies failed: {:?}", e);
                            // Still emit success since file storage worked
                            let _ = app_handle.emit("twitch-login-complete", ());
                        }
                        (Err(e), Ok(_)) => {
                            error!("[LOGIN] Token saved to cookies but file failed: {:?}", e);
                            // Still emit success since cookies worked
                            let _ = app_handle.emit("twitch-login-complete", ());
                        }
                        (Err(file_err), Err(cookie_err)) => {
                            error!(
                                "[LOGIN] Failed to store token anywhere! File: {:?}, Cookie: {:?}",
                                file_err, cookie_err
                            );
                            let _ = app_handle.emit(
                                "twitch-login-error",
                                format!("Failed to store token: {}", file_err),
                            );
                        }
                    }
                }
                Err(e) => {
                    error!("[LOGIN] Token polling failed: {}", e);
                    let _ = app_handle.emit("twitch-login-error", e.to_string());
                }
            }
        });

        // Return the verification URI and the matching user code for the frontend
        // to open/display. Both come from the single device flow polled above.
        Ok((verification_uri, user_code))
    }

    // Device code flow methods (kept for backward compatibility if needed)
    pub async fn start_device_login(_state: &AppState) -> Result<DeviceCodeInfo> {
        let client = crate::services::http::client().clone();
        let device_response = Self::start_device_flow(&client).await?;

        Ok(DeviceCodeInfo {
            user_code: device_response.user_code,
            verification_uri: device_response.verification_uri,
            device_code: device_response.device_code,
            interval: device_response.interval,
            expires_in: device_response.expires_in,
        })
    }

    pub async fn complete_device_login(device_code: &str, _state: &AppState) -> Result<String> {
        let client = crate::services::http::client().clone();

        let token_response = Self::poll_for_token(&client, device_code, 5, 1800).await?;

        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        let storable_token = StorableToken {
            access_token: token_response.access_token.clone(),
            refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
            expires_at: expires_at.timestamp(),
        };

        // Store token to file (primary storage)
        Self::store_token_to_file(&storable_token)?;

        // Also try to store in keyring as backup
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
            let token_json = serde_json::to_string(&storable_token)?;
            let _ = entry.set_password(&token_json);
        }

        // Log for debugging
        debug!(
            "Token stored successfully in keyring. Service: {}, Username: {}",
            KEYRING_SERVICE, KEYRING_USERNAME
        );
        debug!(
            "Access token: {}...",
            &token_response.access_token[..10.min(token_response.access_token.len())]
        );

        Ok("Login successful".to_string())
    }

    /// Build the Twitch OAuth authorize URL for linking a NEW account in the
    /// system browser. `force_verify=true` makes Twitch always show the account
    /// chooser / login, so the user signs in deliberately as a different account
    /// rather than silently reusing an existing browser session. `state` is an
    /// opaque CSRF token the caller verifies against the redirect. This reads
    /// nothing from the primary's cached token.
    pub(crate) fn build_authorize_url(state: &str) -> Result<String> {
        let url = reqwest::Url::parse_with_params(
            "https://id.twitch.tv/oauth2/authorize",
            &[
                ("client_id", CLIENT_ID),
                ("redirect_uri", REDIRECT_URI),
                ("response_type", "code"),
                ("scope", SCOPES),
                ("force_verify", "true"),
                ("state", state),
            ],
        )?;
        Ok(url.to_string())
    }

    /// Exchange an authorization code (from the redirect) for a fresh token.
    /// Deliberately does NOT persist anything: the caller decides where the token
    /// goes. For a linked secondary account that is the AccountStore's secondary
    /// slot, never the primary's cache.
    pub(crate) async fn exchange_code_for_token(code: &str) -> Result<StorableToken> {
        let client = crate::services::http::client().clone();
        let params = [
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", REDIRECT_URI),
        ];

        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Token exchange failed: {}", error_text));
        }

        let token_response: TokenResponse = response.json().await?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        Ok(StorableToken {
            access_token: token_response.access_token,
            refresh_token: token_response.refresh_token.unwrap_or_default(),
            expires_at: expires_at.timestamp(),
        })
    }

    async fn start_device_flow(client: &Client) -> Result<DeviceCodeResponse> {
        let params = [("client_id", CLIENT_ID), ("scopes", SCOPES)];

        let response = client
            .post("https://id.twitch.tv/oauth2/device")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to start device flow: {}",
                error_text
            ));
        }

        let device_response: DeviceCodeResponse = response.json().await?;
        Ok(device_response)
    }

    async fn poll_for_token(
        client: &Client,
        device_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<TokenResponse> {
        let start_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let expiry_time = start_time + expires_in;
        let mut poll_interval = interval;

        loop {
            let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            if current_time >= expiry_time {
                return Err(anyhow::anyhow!(
                    "Device code expired. Please try logging in again."
                ));
            }

            tokio::time::sleep(Duration::from_secs(poll_interval)).await;

            let params = [
                ("client_id", CLIENT_ID),
                ("scopes", SCOPES),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ];

            let response = client
                .post("https://id.twitch.tv/oauth2/token")
                .form(&params)
                .send()
                .await?;

            if response.status().is_success() {
                let token_response: TokenResponse = response.json().await?;
                return Ok(token_response);
            }

            let error_text = response.text().await?;

            if error_text.contains("authorization_pending") {
                // User hasn't authorized yet, continue polling
                continue;
            } else if error_text.contains("slow_down") {
                // Twitch wants us to slow down
                poll_interval += 2;
                continue;
            } else if error_text.contains("expired_token") {
                return Err(anyhow::anyhow!(
                    "Device code expired. Please try logging in again."
                ));
            } else {
                return Err(anyhow::anyhow!("Token polling failed: {}", error_text));
            }
        }
    }

    pub async fn logout() -> Result<()> {
        // Delete token from file storage
        let _ = Self::delete_token_file();

        // Delete cookies
        let _ = Self::delete_cookies().await;

        // Also try to delete from all known keyring locations
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
            let _ = entry.delete_credential();
        }

        if let Ok(entry) = Entry::new("StreamNook", "twitch_token") {
            let _ = entry.delete_credential();
        }

        if let Ok(entry) = Entry::new("streamnook", "twitch_token") {
            let _ = entry.delete_credential();
        }

        debug!("[LOGOUT] Complete - all tokens cleared from all storage locations");
        Ok(())
    }

    pub(crate) async fn refresh_token(refresh_token: &str) -> Result<StorableToken> {
        let client = crate::services::http::client().clone();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
        ];

        debug!("[REFRESH] Attempting to refresh token...");

        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Failed to refresh token: {}", error_text));
        }

        let token_response: TokenResponse = response.json().await?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        let new_storable_token = StorableToken {
            access_token: token_response.access_token,
            refresh_token: token_response
                .refresh_token
                .unwrap_or_else(|| refresh_token.to_string()),
            expires_at: expires_at.timestamp(),
        };

        // Pure transform: the caller decides where this token is stored. The
        // primary callers below persist it via `persist_primary_token`; the
        // secondary-account path (`AccountStore::get_token_for`) persists it only
        // to that account's own store. Persisting here would write EVERY refresh
        // into the primary slot, including a secondary's, which silently swaps
        // the main account out from under the user.
        Ok(new_storable_token)
    }

    pub async fn get_token() -> Result<String> {
        // Serialized end to end. Two callers landing inside the 5-minute
        // expiry buffer would otherwise BOTH refresh, and whichever persists
        // last can write back a stale refresh token, killing the session until
        // the user logs in again. The no-refresh path this also serializes is
        // a file read, so contention costs nothing. (On Android the background
        // notification worker shares this path with the running app, which
        // widened the race enough to be worth closing.)
        static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _guard = REFRESH_LOCK.lock().await;

        // Try to load from file first (primary storage)
        match Self::load_token_from_file() {
            Ok(mut token) => {
                // debug!("[GET_TOKEN] Token retrieved from file storage");

                // Check if token is expired or about to expire (within 5 minutes)
                let buffer_time = 300; // 5 minutes buffer
                if Utc::now().timestamp() >= (token.expires_at - buffer_time) {
                    // Token is expired or about to expire, refresh it
                    if !token.refresh_token.is_empty() {
                        debug!("[GET_TOKEN] Token expired or expiring soon, refreshing...");
                        match Self::refresh_token(&token.refresh_token).await {
                            Ok(new_token) => {
                                token = new_token;
                                // Persist the refreshed primary token everywhere
                                // (file + cookies + keyring).
                                let _ = Self::persist_primary_token(&token).await;
                            }
                            Err(e) => {
                                error!("[GET_TOKEN] Failed to refresh token: {:?}", e);
                                return Err(anyhow::anyhow!(
                                    "Token expired and refresh failed. Please log in again."
                                ));
                            }
                        }
                    } else {
                        return Err(anyhow::anyhow!(
                            "Token expired and no refresh token available. Please log in again."
                        ));
                    }
                }

                return Ok(token.access_token);
            }
            Err(file_err) => {
                debug!("[GET_TOKEN] Could not read from file: {:?}", file_err);

                // Try cookies as fallback (new persistent storage)
                debug!("[GET_TOKEN] Trying cookies as fallback...");
                match Self::load_token_from_cookies().await {
                    Ok(mut cookie_token) => {
                        debug!("[GET_TOKEN] Token retrieved from cookies");

                        // Check if token is expired or about to expire
                        let buffer_time = 300; // 5 minutes buffer
                        if cookie_token.expires_at > 0
                            && Utc::now().timestamp() >= (cookie_token.expires_at - buffer_time)
                        {
                            // Try to refresh if we have a refresh token
                            if !cookie_token.refresh_token.is_empty() {
                                debug!(
                                    "[GET_TOKEN] Cookie token expired or expiring soon, refreshing..."
                                );
                                match Self::refresh_token(&cookie_token.refresh_token).await {
                                    Ok(new_token) => {
                                        cookie_token = new_token.clone();
                                        // Persist to all primary-slot storage.
                                        let _ = Self::persist_primary_token(&new_token).await;
                                    }
                                    Err(e) => {
                                        error!(
                                            "[GET_TOKEN] Failed to refresh cookie token: {:?}",
                                            e
                                        );
                                        let _ = Self::delete_cookies().await;
                                        return Err(anyhow::anyhow!(
                                            "Token expired and refresh failed. Please log in again."
                                        ));
                                    }
                                }
                            } else {
                                // No refresh token, validate the token directly
                                let client = crate::services::http::client().clone();
                                let response = client
                                    .get("https://id.twitch.tv/oauth2/validate")
                                    .header(
                                        "Authorization",
                                        format!("OAuth {}", cookie_token.access_token),
                                    )
                                    .send()
                                    .await?;

                                if response.status() == 401 {
                                    debug!("[GET_TOKEN] Cookie token is invalid, clearing cookies");
                                    let _ = Self::delete_cookies().await;
                                    return Err(anyhow::anyhow!(
                                        "Not authenticated. Please log in to Twitch first."
                                    ));
                                }
                            }
                        }

                        // Save to file for next time
                        let _ = Self::store_token_to_file(&cookie_token);

                        return Ok(cookie_token.access_token);
                    }
                    Err(_) => {
                        // Fallback to keyring if cookies don't exist
                        debug!("[GET_TOKEN] Trying keyring as fallback...");

                        if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
                            if let Ok(pwd) = entry.get_password() {
                                debug!("[GET_TOKEN] Token retrieved from keyring fallback");

                                let mut token: StorableToken = match serde_json::from_str(&pwd) {
                                    Ok(t) => t,
                                    Err(e) => {
                                        error!(
                                            "[GET_TOKEN] Failed to parse keyring token: {:?}",
                                            e
                                        );
                                        return Err(anyhow::anyhow!(
                                            "Not authenticated. Please log in to Twitch first."
                                        ));
                                    }
                                };

                                // Save it to file and cookies for next time
                                let _ = Self::store_token_to_file(&token);
                                let _ = Self::store_token_to_cookies(&token).await;

                                // Check if token needs refresh
                                let buffer_time = 300;
                                if Utc::now().timestamp() >= (token.expires_at - buffer_time) {
                                    if !token.refresh_token.is_empty() {
                                        debug!("[GET_TOKEN] Keyring token expired, refreshing...");
                                        match Self::refresh_token(&token.refresh_token).await {
                                            Ok(new_token) => {
                                                token = new_token;
                                                let _ = Self::persist_primary_token(&token).await;
                                            }
                                            Err(e) => {
                                                error!(
                                                    "[GET_TOKEN] Failed to refresh keyring token: {:?}",
                                                    e
                                                );
                                                return Err(anyhow::anyhow!(
                                                    "Token expired and refresh failed. Please log in again."
                                                ));
                                            }
                                        }
                                    }
                                }

                                return Ok(token.access_token);
                            }
                        }

                        error!("[GET_TOKEN] No token found in file, cookies, or keyring storage");
                        Err(anyhow::anyhow!(
                            "Not authenticated. Please log in to Twitch first."
                        ))
                    }
                }
            }
        }
    }

    /// Verify the current token's health and return detailed status
    /// This should be called on app startup to proactively check/refresh the token
    pub async fn verify_token_health() -> Result<TokenHealthStatus> {
        let client = crate::services::http::client().clone();

        // Try to get the current token (this will auto-refresh if needed)
        let access_token = match Self::get_token().await {
            Ok(t) => t,
            Err(e) => {
                debug!("[Auth Debug] No valid token available: {:?}", e);
                return Ok(TokenHealthStatus {
                    is_valid: false,
                    seconds_remaining: 0,
                    hours_remaining: 0,
                    minutes_remaining: 0,
                    scopes: vec![],
                    user_id: None,
                    login: None,
                    needs_refresh: false,
                    error: Some(e.to_string()),
                });
            }
        };

        // Validate the token with Twitch
        let response = client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", access_token))
            .send()
            .await?;

        if !response.status().is_success() {
            debug!("[Auth Debug] Token is INVALID or EXPIRED. User needs to login or refresh.");
            return Ok(TokenHealthStatus {
                is_valid: false,
                seconds_remaining: 0,
                hours_remaining: 0,
                minutes_remaining: 0,
                scopes: vec![],
                user_id: None,
                login: None,
                needs_refresh: true,
                error: Some("Token validation failed".to_string()),
            });
        }

        let data: serde_json::Value = response.json().await?;

        let seconds_remaining = data["expires_in"].as_i64().unwrap_or(0);
        let hours = seconds_remaining / 3600;
        let minutes = (seconds_remaining % 3600) / 60;

        let scopes: Vec<String> = data["scopes"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let required_scopes: Vec<&str> = SCOPES.split_whitespace().collect();
        let missing_scopes: Vec<&str> = required_scopes
            .into_iter()
            .filter(|&required| !scopes.iter().any(|s| s == required))
            .collect();

        if !missing_scopes.is_empty() {
            // INFO on purpose: this line is the only evidence of a forced
            // re-login in the shipped log, and a scope Twitch will not grant
            // would otherwise loop the user through login forever.
            log::warn!(
                "[Auth] Token is missing required scopes {:?} (token has {} scopes); clearing accounts for re-login",
                missing_scopes,
                scopes.len()
            );

            // Clear the token AND the account registry. A scopes upgrade
            // invalidates every stored token at once, so this is a full re-auth,
            // not a single-account sign-out (no promotion). Emptying the registry
            // is load-bearing: it forces the subsequent login to open in the
            // DEFAULT WebView2 profile, the only profile `TwitchAuthService::
            // get_token` reads. If the registry survived, the re-login would
            // reopen in the old account's per-account profile and the new
            // web-session cookie would be invisible to the stream resolver — the
            // "non-proxy mode requires a twitch login" dead-end even though the
            // user just logged in.
            crate::services::account_store::AccountStore::reset_all().await;

            return Ok(TokenHealthStatus {
                is_valid: false,
                seconds_remaining: 0,
                hours_remaining: 0,
                minutes_remaining: 0,
                scopes,
                user_id: None,
                login: None,
                needs_refresh: false,
                error: Some(format!(
                    "Missing scopes: {}. Please log in again.",
                    missing_scopes.join(", ")
                )),
            });
        }

        let user_id = data["user_id"].as_str().map(|s| s.to_string());
        let login = data["login"].as_str().map(|s| s.to_string());

        debug!("[Auth Debug] Token is VALID and has all required scopes.");
        debug!("[Auth Debug] Scopes: {}", scopes.join(", "));
        debug!(
            "[Auth Debug] Time remaining: {}h {}m ({}s)",
            hours, minutes, seconds_remaining
        );

        let needs_refresh = seconds_remaining < 3600;
        if needs_refresh {
            debug!("[Auth Debug] Token expires in less than 1 hour! Consider refreshing soon.");
        }

        Ok(TokenHealthStatus {
            is_valid: true,
            seconds_remaining,
            hours_remaining: hours,
            minutes_remaining: minutes,
            scopes,
            user_id,
            login,
            needs_refresh,
            error: None,
        })
    }

    /// Force refresh the token even if it hasn't expired yet
    pub async fn force_refresh_token() -> Result<String> {
        // Try to load token from any storage
        let token = Self::load_token_from_file()
            .or_else(|_| {
                // Try synchronous approach for cookies
                futures::executor::block_on(Self::load_token_from_cookies())
            })
            .or_else(|_| {
                // Try keyring
                if let Ok(entry) = Entry::new(KEYRING_SERVICE, KEYRING_USERNAME) {
                    if let Ok(pwd) = entry.get_password() {
                        return serde_json::from_str::<StorableToken>(&pwd).map_err(|e| {
                            anyhow::anyhow!("Failed to parse keyring token: {:?}", e)
                        });
                    }
                }
                Err(anyhow::anyhow!("No token found in any storage"))
            })?;

        if token.refresh_token.is_empty() {
            return Err(anyhow::anyhow!(
                "No refresh token available. Please log in again."
            ));
        }

        debug!("[Auth Debug] Force refreshing token...");
        let new_token = Self::refresh_token(&token.refresh_token).await?;

        // This is an explicit PRIMARY refresh, so persist to all primary-slot
        // storage (file + cookies + keyring).
        let _ = Self::persist_primary_token(&new_token).await;

        debug!("[Auth Debug] Token refreshed successfully!");
        Ok(new_token.access_token)
    }

    pub async fn get_followed_streams(_state: &AppState) -> Result<Vec<TwitchStream>> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Not authenticated. Please log in to Twitch first."
                ));
            }
        };
        let client = crate::services::http::client().clone();

        // First, get the user ID
        let user_response = client
            .get("https://api.twitch.tv/helix/users")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let user_id = user_response["data"][0]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to get user ID"))?;

        // Now get followed streams with user_id
        let response = client
            .get(format!(
                "https://api.twitch.tv/helix/streams/followed?user_id={}",
                user_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header(ACCEPT, "application/json")
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Handle null or missing data field
        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        // Create maps of user_id -> broadcaster_type and user_id -> profile image.
                        // The followed-streams endpoint only returns a stream-preview thumbnail
                        // (with {width}x{height} placeholders), so the real avatar has to come from
                        // this users batch — which we already fetch for broadcaster_type anyway.
                        let mut broadcaster_types = std::collections::HashMap::new();
                        let mut profile_images = std::collections::HashMap::new();
                        for user in users_data {
                            if let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                                if let Some(broadcaster_type) =
                                    user.get("broadcaster_type").and_then(|v| v.as_str())
                                {
                                    if !broadcaster_type.is_empty() {
                                        broadcaster_types
                                            .insert(id.to_string(), broadcaster_type.to_string());
                                    }
                                }
                                if let Some(profile_image_url) =
                                    user.get("profile_image_url").and_then(|v| v.as_str())
                                {
                                    if !profile_image_url.is_empty() {
                                        profile_images
                                            .insert(id.to_string(), profile_image_url.to_string());
                                    }
                                }
                            }
                        }

                        // Update streams with broadcaster types and profile images
                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                            if let Some(profile_image_url) = profile_images.get(&stream.user_id) {
                                stream.profile_image_url = Some(profile_image_url.clone());
                            }
                        }
                    }
                }

                Ok(streams)
            }
            None => Ok(Vec::new()), // Return empty vec if no data
        }
    }

    pub async fn get_channel_info(channel_name: &str, _state: &AppState) -> Result<ChannelInfo> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => {
                return Err(anyhow::anyhow!(
                    "Not authenticated. Please log in to Twitch first."
                ));
            }
        };
        // Helix /channels accepts broadcaster_id ONLY. There is no
        // broadcaster_login parameter, so querying by login always came back
        // with an empty `data` array and surfaced as "Channel not found" for
        // channels that plainly exist. That failure is why a channel the user
        // does not follow rendered chat with no Twitch badges: without an id
        // the badge metadata cache is never populated for the room, so every
        // badge resolves to no image. Resolve the login to an id first.
        let broadcaster_id = Self::get_user_by_login(channel_name).await?.id;
        let client = crate::services::http::client().clone();
        let response = client
            .get(format!(
                "https://api.twitch.tv/helix/channels?broadcaster_id={}",
                broadcaster_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Check if data exists and has at least one element
        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("Channel '{}' not found", channel_name))?;

        let info: ChannelInfo = serde_json::from_value(data.clone())?;
        Ok(info)
    }

    /// Create a clip of a live broadcast via Helix `POST /clips` (needs the
    /// `clips:edit` scope). Returns `(clip_id, edit_url)`. The clip captures the
    /// last ~30s and is processed async, so it may take a few seconds before its
    /// thumbnail/playback is ready (the public URL `clips.twitch.tv/<id>` works
    /// once processed). Documented statuses are mapped to short codes the UI
    /// turns into friendly messages, rather than leaking raw HTTP errors.
    pub async fn create_clip(broadcaster_id: &str) -> Result<(String, String)> {
        let token = Self::get_token()
            .await
            .map_err(|_| anyhow::anyhow!("REAUTH"))?;
        let client = crate::services::http::client().clone();
        let resp = client
            .post("https://api.twitch.tv/helix/clips")
            .query(&[("broadcaster_id", broadcaster_id)])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("NETWORK"))?;

        let status = resp.status();
        if status.is_success() {
            let json: serde_json::Value =
                resp.json().await.map_err(|_| anyhow::anyhow!("ERROR"))?;
            let data = json
                .get("data")
                .and_then(|d| d.as_array())
                .and_then(|a| a.first())
                .ok_or_else(|| anyhow::anyhow!("ERROR"))?;
            let id = data
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let edit_url = data
                .get("edit_url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                return Err(anyhow::anyhow!("ERROR"));
            }
            return Ok((id, edit_url));
        }

        // Short codes the frontend maps to messages (see createClip in AppStore).
        let code = match status.as_u16() {
            401 => "REAUTH",    // token lacks clips:edit → re-login needed
            403 => "FORBIDDEN", // channel restricts who can clip
            400 => "DISABLED",  // clips disabled for this channel
            404 => "NOTFOUND",
            422 => "OFFLINE", // can only clip a live broadcast
            _ => "ERROR",
        };
        Err(anyhow::anyhow!("{}", code))
    }

    /// Fetch the current live broadcast's stream id + start time via Helix Get
    /// Streams. The stream `id` equals the GQL `broadcastID` that `CreateRawMedia`
    /// wants for a live clip (confirmed by capture: `clip.broadcaster.stream.id`
    /// == the `broadcastID`), and `started_at` lets the caller derive the live
    /// offset (uptime). Errors "OFFLINE" when the channel isn't live.
    pub async fn get_live_broadcast(broadcaster_id: &str) -> Result<(String, String)> {
        let token = Self::get_token()
            .await
            .map_err(|_| anyhow::anyhow!("REAUTH"))?;
        let client = crate::services::http::client().clone();
        let resp = client
            .get("https://api.twitch.tv/helix/streams")
            .query(&[("user_id", broadcaster_id)])
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("NETWORK"))?;
        if !resp.status().is_success() {
            let code = if resp.status().as_u16() == 401 {
                "REAUTH"
            } else {
                "ERROR"
            };
            return Err(anyhow::anyhow!("{}", code));
        }
        let json: serde_json::Value = resp.json().await.map_err(|_| anyhow::anyhow!("ERROR"))?;
        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| anyhow::anyhow!("OFFLINE"))?;
        let id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let started_at = data
            .get("started_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Err(anyhow::anyhow!("OFFLINE"));
        }
        Ok((id, started_at))
    }

    pub async fn get_user_info() -> Result<UserInfo> {
        let token = Self::get_token().await?;
        Self::get_user_info_with_token(&token).await
    }

    /// Fetch the Helix user profile for an arbitrary access token, not just the
    /// primary login's. Used by the multi-account store to identify a freshly
    /// linked account from its own token.
    pub(crate) async fn get_user_info_with_token(token: &str) -> Result<UserInfo> {
        let client = crate::services::http::client().clone();

        let response = client
            .get("https://api.twitch.tv/helix/users")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("Failed to get user info"))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    /// Send a chat message via Helix Send Chat Message. Returns
    /// (message_id, is_sent, drop_reason). The message_id is the AUTHORITATIVE id
    /// Twitch assigns, so an own message can be deleted immediately without
    /// waiting to catch its IRC echo. `is_sent` is false when Twitch accepted the
    /// request but dropped the message (e.g. AutoMod), with `drop_reason` set.
    /// Requires the `user:write:chat` scope; callers fall back to IRC on error.
    pub async fn send_chat_message_helix(
        broadcaster_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_msg_id: Option<&str>,
    ) -> Result<(Option<String>, bool, Option<String>)> {
        let token = Self::get_token().await?;
        Self::send_chat_message_helix_with_token(
            &token,
            broadcaster_id,
            sender_id,
            message,
            reply_parent_msg_id,
        )
        .await
    }

    /// Same as `send_chat_message_helix`, but with an explicit token so a chosen
    /// secondary account can send as itself. The caller passes that account's
    /// token plus its own user id as `sender_id`.
    pub async fn send_chat_message_helix_with_token(
        token: &str,
        broadcaster_id: &str,
        sender_id: &str,
        message: &str,
        reply_parent_msg_id: Option<&str>,
    ) -> Result<(Option<String>, bool, Option<String>)> {
        let client = crate::services::http::client().clone();

        let mut body = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "sender_id": sender_id,
            "message": message,
        });
        if let Some(reply) = reply_parent_msg_id {
            body["reply_parent_message_id"] = serde_json::Value::String(reply.to_string());
        }

        let response = client
            .post("https://api.twitch.tv/helix/chat/messages")
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let json = response
            .json::<serde_json::Value>()
            .await
            .unwrap_or(serde_json::Value::Null);
        if !status.is_success() {
            return Err(anyhow::anyhow!(
                "Helix send returned HTTP {}: {}",
                status.as_u16(),
                json
            ));
        }

        let data = json
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first());
        let message_id = data
            .and_then(|d| d.get("message_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let is_sent = data
            .and_then(|d| d.get("is_sent"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let drop_reason = data
            .and_then(|d| d.get("drop_reason"))
            .and_then(|dr| {
                dr.get("message")
                    .or_else(|| dr.get("code"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);
        Ok((message_id, is_sent, drop_reason))
    }

    pub async fn get_user_by_login(login: &str) -> Result<UserInfo> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let response = client
            .get(format!("https://api.twitch.tv/helix/users?login={}", login))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("User '{}' not found", login))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    pub async fn get_user_by_id(user_id: &str) -> Result<UserInfo> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let response = client
            .get(format!("https://api.twitch.tv/helix/users?id={}", user_id))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| anyhow::anyhow!("User with ID '{}' not found", user_id))?;

        let user_info: UserInfo = serde_json::from_value(data.clone())?;
        Ok(user_info)
    }

    /// Discover feed dispatcher. First page (no cursor) is Twitch's real
    /// recommendation feed via GQL; infinite scroll continues into the Helix
    /// top-streams list behind a "helix:"-prefixed cursor. The cursor stays an
    /// opaque string to the frontend, so its pagination contract is unchanged.
    pub async fn get_recommended_streams_paginated(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
        languages: Vec<String>,
        personalized: bool,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        match cursor {
            None => match Self::fetch_gql_recommended(40, &languages, personalized).await {
                Ok(streams) if !streams.is_empty() => {
                    debug!("[Discovery] recommendations page: {} streams", streams.len());
                    Ok((streams, Some("helix:".to_string())))
                }
                // Empty (including filtered-to-empty) or failed recommendations
                // fall through to Helix in the same call: an empty first page
                // could never trigger the scroll-based load-more.
                Ok(_) => {
                    debug!("[Discovery] recommendations empty, serving Helix top streams");
                    Self::fetch_helix_top(_state, None, limit, &languages).await
                }
                Err(e) => {
                    warn!("[Discovery] recommendations failed ({}), serving Helix top streams", e);
                    Self::fetch_helix_top(_state, None, limit, &languages).await
                }
            },
            Some(c) => {
                let after = c.strip_prefix("helix:").unwrap_or(&c);
                let after = if after.is_empty() {
                    None
                } else {
                    Some(after.to_string())
                };
                Self::fetch_helix_top(_state, after, limit, &languages).await
            }
        }
    }

    /// Twitch's recommendation feed (GQL `recommendedStreams`). With
    /// `personalized` (an opt-in setting, off by default) it tries the
    /// account-personalized form first (drops token + Android client, the
    /// proven authorized-GQL pairing); any rejection is remembered for the
    /// session so the wasted round-trip is paid at most once per app run.
    /// Otherwise, and as the fallback, it makes the anonymous web-client call,
    /// which is device/geo-aware via the stable X-Device-Id and never sends
    /// account credentials.
    async fn fetch_gql_recommended(
        limit: u32,
        languages: &[String],
        personalized: bool,
    ) -> Result<Vec<TwitchStream>> {
        use std::sync::atomic::{AtomicBool, Ordering};
        static AUTH_RECS_REJECTED: AtomicBool = AtomicBool::new(false);

        let client = crate::services::http::client().clone();
        // recRequestID must be typed ID!, not String! (String! fails GQL
        // validation even though the value is a plain string).
        let query = "query($first: Int!, $recRequestID: ID!) { \
            recommendedStreams(first: $first, recRequestID: $recRequestID, location: \"LEFT_NAV\", \
                context: { platform: \"web\", clientApp: \"twilight\" }) { \
                edges { node { \
                    id title viewersCount language previewImageURL createdAt \
                    freeformTags { name } \
                    game { id name displayName } \
                    broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } \
                } } \
            } }";
        let body = serde_json::json!({
            "query": query,
            "variables": {
                "first": limit,
                "recRequestID": uuid::Uuid::new_v4().to_string().replace('-', ""),
            }
        });
        let device_id = gql_device_id();

        let mut json: Option<serde_json::Value> = None;
        if personalized && !AUTH_RECS_REJECTED.load(Ordering::Relaxed) {
            if let Ok(token) =
                crate::services::drops_auth_service::DropsAuthService::get_token().await
            {
                let attempt = tokio::time::timeout(
                    Duration::from_secs(4),
                    client
                        .post("https://gql.twitch.tv/gql")
                        .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
                        .header(AUTHORIZATION, format!("OAuth {}", token))
                        .header("Origin", "https://www.twitch.tv")
                        .header("Referer", "https://www.twitch.tv")
                        .header("X-Device-Id", &device_id)
                        .header(
                            "Client-Session-Id",
                            uuid::Uuid::new_v4().to_string().replace('-', ""),
                        )
                        .json(&body)
                        .send(),
                )
                .await;
                match attempt {
                    Ok(Ok(resp)) => {
                        let status = resp.status();
                        match resp.json::<serde_json::Value>().await {
                            Ok(parsed) => {
                                let has_edges = parsed
                                    .pointer("/data/recommendedStreams/edges")
                                    .and_then(|e| e.as_array())
                                    .map(|a| !a.is_empty())
                                    .unwrap_or(false);
                                if status.is_success()
                                    && parsed.get("errors").is_none()
                                    && has_edges
                                {
                                    debug!("[Discovery] authorized recommendations resolved");
                                    json = Some(parsed);
                                } else {
                                    // Only a real rejection disables the attempt for
                                    // the session; transient emptiness does not.
                                    if status.is_client_error() || parsed.get("errors").is_some() {
                                        warn!(
                                            "[Discovery] authorized recommendations rejected (HTTP {}): {:?}",
                                            status,
                                            parsed.get("errors")
                                        );
                                        AUTH_RECS_REJECTED.store(true, Ordering::Relaxed);
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("[Discovery] authorized recommendations parse failed: {}", e)
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        debug!("[Discovery] authorized recommendations request failed: {}", e)
                    }
                    Err(_) => debug!("[Discovery] authorized recommendations timed out"),
                }
            }
        }

        let json = match json {
            Some(j) => j,
            None => {
                let resp = client
                    .post("https://gql.twitch.tv/gql")
                    .header("Client-Id", TWITCH_GQL_CLIENT_ID)
                    .header("X-Device-ID", &device_id)
                    .json(&body)
                    .send()
                    .await?
                    .json::<serde_json::Value>()
                    .await?;
                if let Some(errors) = resp.get("errors") {
                    return Err(anyhow::anyhow!("recommendedStreams GQL errors: {}", errors));
                }
                debug!("[Discovery] anonymous recommendations resolved");
                resp
            }
        };

        let mut streams = json
            .pointer("/data/recommendedStreams/edges")
            .and_then(|e| e.as_array())
            .map(|e| Self::map_recommended_edges(e))
            .unwrap_or_default();

        if !languages.is_empty() {
            let wanted: Vec<String> = languages.iter().map(|l| l.to_ascii_lowercase()).collect();
            streams.retain(|s| {
                s.language
                    .as_deref()
                    .map(|l| wanted.iter().any(|w| w == l))
                    .unwrap_or(false)
            });
        }

        Ok(streams)
    }

    /// Map GQL `recommendedStreams` edges into the app's stream shape,
    /// mirroring the get_streams_by_game_with_tags mapping.
    fn map_recommended_edges(edges: &[serde_json::Value]) -> Vec<TwitchStream> {
        let mut streams: Vec<TwitchStream> = Vec::new();
        {
            for edge in edges {
                let node = match edge.get("node") {
                    Some(n) if !n.is_null() => n,
                    _ => continue,
                };
                let broadcaster = match node.get("broadcaster") {
                    Some(b) if !b.is_null() => b,
                    _ => continue,
                };

                let user_login = broadcaster
                    .get("login")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let user_name = broadcaster
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&user_login)
                    .to_string();
                let roles = broadcaster.get("roles");
                let broadcaster_type = if roles
                    .and_then(|r| r.get("isPartner"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("partner".to_string())
                } else if roles
                    .and_then(|r| r.get("isAffiliate"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("affiliate".to_string())
                } else {
                    None
                };

                let stream_tags: Vec<String> = node
                    .get("freeformTags")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| {
                                t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let game = node.get("game").filter(|g| !g.is_null());
                let game_name = game
                    .and_then(|g| {
                        g.get("displayName")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .or_else(|| g.get("name").and_then(|v| v.as_str()))
                    })
                    .unwrap_or("")
                    .to_string();

                streams.push(TwitchStream {
                    id: node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_id: broadcaster
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    user_name,
                    user_login,
                    title: node.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    viewer_count: node
                        .get("viewersCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    game_id: game
                        .and_then(|g| g.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    game_name,
                    thumbnail_url: node
                        .get("previewImageURL")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    started_at: node
                        .get("createdAt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    broadcaster_type,
                    has_shared_chat: None,
                    profile_image_url: broadcaster
                        .get("profileImageURL")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    is_live: Some(true),
                    tags: if stream_tags.is_empty() { None } else { Some(stream_tags) },
                    language: node
                        .get("language")
                        .and_then(|v| v.as_str())
                        .map(normalize_gql_language),
                });
            }
        }
        streams
    }

    /// Helix top-live-streams list (the pre-recommendations Discover source),
    /// now with an optional broadcast-language filter. Cursors returned from
    /// here carry the "helix:" phase prefix.
    async fn fetch_helix_top(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
        languages: &[String],
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        // Try to get token, but don't fail if not authenticated
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        // Build URL with pagination
        let mut url = format!("https://api.twitch.tv/helix/streams?first={}", limit);
        if let Some(cursor) = cursor {
            url.push_str(&format!("&after={}", cursor));
        }
        for lang in languages {
            if lang
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                url.push_str(&format!("&language={}", lang.to_ascii_lowercase()));
            }
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        // Add authorization if we have a token
        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        // Get pagination cursor
        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        // Handle null or missing data field
        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        // Create a map of user_id -> broadcaster_type
                        let mut broadcaster_types = std::collections::HashMap::new();
                        for user in users_data {
                            if let (Some(id), Some(broadcaster_type)) = (
                                user.get("id").and_then(|v| v.as_str()),
                                user.get("broadcaster_type").and_then(|v| v.as_str()),
                            ) {
                                if !broadcaster_type.is_empty() {
                                    broadcaster_types
                                        .insert(id.to_string(), broadcaster_type.to_string());
                                }
                            }
                        }

                        // Update streams with broadcaster types
                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                        }
                    }
                }

                Ok((streams, next_cursor.map(|c| format!("helix:{}", c))))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_recommended_streams(_state: &AppState) -> Result<Vec<TwitchStream>> {
        let (streams, _) =
            Self::get_recommended_streams_paginated(_state, None, 20, Vec::new(), false).await?;

        Ok(streams)
    }

    async fn populate_shared_chat_status(streams: &mut Vec<TwitchStream>) {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => return, // Can't check without token
        };

        let client = crate::services::http::client().clone();

        for stream in streams.iter_mut() {
            // Check if this broadcaster is in a shared chat session
            let url = format!(
                "https://api.twitch.tv/helix/chat/shared?broadcaster_id={}",
                stream.user_id
            );

            match client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(response) => {
                    // 200 OK means they're in a shared chat session
                    // 404 means they're not
                    stream.has_shared_chat = Some(response.status().is_success());
                }
                Err(_) => {
                    stream.has_shared_chat = Some(false);
                }
            }
        }
    }

    pub async fn get_top_games(_state: &AppState, limit: u32) -> Result<Vec<serde_json::Value>> {
        let (games, _) = Self::get_top_games_paginated(_state, None, limit).await?;
        Ok(games)
    }

    pub async fn get_top_games_paginated(
        _state: &AppState,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<serde_json::Value>, Option<String>)> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut url = format!("https://api.twitch.tv/helix/games/top?first={}", limit);
        if let Some(cursor) = cursor {
            url.push_str(&format!("&after={}", cursor));
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        // Get pagination cursor
        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                // Each top game needs a viewer-count total, which comes from a
                // second Helix call per game (streams?game_id=...). Fire them
                // concurrently (bounded) instead of one-at-a-time: a page of 40
                // games was doing 40 sequential round-trips, which is what made
                // the Categories view take many seconds to load. `buffered` keeps
                // the original top-games order while capping in-flight requests.
                use futures_util::StreamExt;
                const MAX_CONCURRENT: usize = 10;

                let mut games_with_viewers: Vec<serde_json::Value> =
                    futures_util::stream::iter(arr.iter().cloned().map(|game| {
                        let client = client.clone();
                        let token = token.clone();
                        async move {
                            let mut game_data = game.clone();

                            if let Some(game_id) = game.get("id").and_then(|id| id.as_str()) {
                                let streams_url = format!(
                                    "https://api.twitch.tv/helix/streams?game_id={}&first=100",
                                    game_id
                                );

                                let mut streams_request =
                                    client.get(&streams_url).header("Client-Id", CLIENT_ID);

                                if let Some(token) = &token {
                                    streams_request = streams_request
                                        .header(AUTHORIZATION, format!("Bearer {}", token));
                                }

                                if let Ok(streams_response) = streams_request.send().await {
                                    if let Ok(streams_json) =
                                        streams_response.json::<serde_json::Value>().await
                                    {
                                        if let Some(streams_data) =
                                            streams_json.get("data").and_then(|d| d.as_array())
                                        {
                                            let total_viewers: i64 = streams_data
                                                .iter()
                                                .filter_map(|s| {
                                                    s.get("viewer_count").and_then(|v| v.as_i64())
                                                })
                                                .sum();

                                            if let Some(obj) = game_data.as_object_mut() {
                                                obj.insert(
                                                    "viewer_count".to_string(),
                                                    serde_json::json!(total_viewers),
                                                );
                                            }
                                        }
                                    }
                                }
                            }

                            game_data
                        }
                    }))
                    .buffered(MAX_CONCURRENT)
                    .collect()
                    .await;

                // Order each page by the viewer totals we just summed, highest
                // first. Twitch's games/top is only roughly viewer-ordered; this
                // makes the Categories grid strictly descending within the page.
                games_with_viewers.sort_by(|a, b| {
                    let va = a.get("viewer_count").and_then(|v| v.as_i64()).unwrap_or(0);
                    let vb = b.get("viewer_count").and_then(|v| v.as_i64()).unwrap_or(0);
                    vb.cmp(&va)
                });

                Ok((games_with_viewers, next_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_streams_by_game(
        _state: &AppState,
        game_id: &str,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/streams?game_id={}&first={}",
            game_id, limit
        );
        if let Some(cursor) = cursor {
            url.push_str(&format!("&after={}", cursor));
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        let mut broadcaster_types = std::collections::HashMap::new();
                        for user in users_data {
                            if let (Some(id), Some(broadcaster_type)) = (
                                user.get("id").and_then(|v| v.as_str()),
                                user.get("broadcaster_type").and_then(|v| v.as_str()),
                            ) {
                                if !broadcaster_type.is_empty() {
                                    broadcaster_types
                                        .insert(id.to_string(), broadcaster_type.to_string());
                                }
                            }
                        }

                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                        }
                    }
                }

                Ok((streams, next_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn search_channels(_state: &AppState, query: &str) -> Result<Vec<TwitchStream>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/search/channels?query={}&first=60&live_only=false",
            urlencoding::encode(query)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                // Convert search results to TwitchStream format
                let mut streams = Vec::new();
                let mut user_ids = Vec::new();

                for channel in arr {
                    if let (
                        Some(id),
                        Some(user_id),
                        Some(user_name),
                        Some(user_login),
                        Some(title),
                    ) = (
                        channel.get("id").and_then(|v| v.as_str()),
                        channel.get("id").and_then(|v| v.as_str()), // Note: Search result 'id' is the user_id
                        channel.get("display_name").and_then(|v| v.as_str()),
                        channel.get("broadcaster_login").and_then(|v| v.as_str()),
                        channel.get("title").and_then(|v| v.as_str()),
                    ) {
                        let game_name = channel
                            .get("game_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let thumbnail_url = channel
                            .get("thumbnail_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let broadcaster_type = channel
                            .get("broadcaster_type")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string());

                        user_ids.push(user_id.to_string());

                        streams.push(TwitchStream {
                            id: id.to_string(), // This is user_id, but search result doesn't provide stream_id
                            user_id: user_id.to_string(),
                            user_name: user_name.to_string(),
                            user_login: user_login.to_string(),
                            title: title.to_string(),
                            viewer_count: 0, // Will be populated from streams API
                            game_id: String::new(), // Will be populated from streams API
                            game_name: game_name.to_string(),
                            thumbnail_url: thumbnail_url.clone(),
                            started_at: channel
                                .get("started_at")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            broadcaster_type,
                            has_shared_chat: None, // Will be populated later
                            profile_image_url: Some(thumbnail_url.clone()), // Preserve the actual profile picture from search
                            is_live: channel.get("is_live").and_then(|v| v.as_bool()),
                            tags: None,
                            language: None,
                        });
                    }
                }

                // Fetch actual stream data to get viewer counts and accurate info
                if !user_ids.is_empty() {
                    // Map of user_id -> live stream data, filled across chunks.
                    let mut stream_data_map: std::collections::HashMap<String, serde_json::Value> =
                        std::collections::HashMap::new();

                    // Helix takes at most 100 `user_id` params per call, and defaults
                    // `first` to 20. Without it a fully-live page of search results
                    // would only enrich its first 20 rows.
                    for chunk in user_ids.chunks(100) {
                        let user_ids_param = chunk
                            .iter()
                            .map(|id| format!("user_id={}", id))
                            .collect::<Vec<_>>()
                            .join("&");

                        let streams_url = format!(
                            "https://api.twitch.tv/helix/streams?{}&first=100",
                            user_ids_param
                        );

                        let mut streams_request =
                            client.get(&streams_url).header("Client-Id", CLIENT_ID);

                        if let Some(token) = &token {
                            streams_request =
                                streams_request.header(AUTHORIZATION, format!("Bearer {}", token));
                        }

                        if let Ok(streams_response) = streams_request.send().await {
                            if let Ok(streams_json) =
                                streams_response.json::<serde_json::Value>().await
                            {
                                if let Some(streams_data) =
                                    streams_json.get("data").and_then(|d| d.as_array())
                                {
                                    for stream_data in streams_data {
                                        if let Some(uid) =
                                            stream_data.get("user_id").and_then(|v| v.as_str())
                                        {
                                            stream_data_map
                                                .insert(uid.to_string(), stream_data.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Update our streams with actual stream data
                    for stream in &mut streams {
                        if let Some(stream_data) = stream_data_map.get(&stream.user_id) {
                            // Update viewer count
                            if let Some(viewer_count) =
                                stream_data.get("viewer_count").and_then(|v| v.as_u64())
                            {
                                stream.viewer_count = viewer_count as u32;
                            }

                            // Update stream ID (actual stream_id, not user_id)
                            if let Some(stream_id) = stream_data.get("id").and_then(|v| v.as_str())
                            {
                                stream.id = stream_id.to_string();
                            }

                            // Update thumbnail URL with actual stream thumbnail
                            if let Some(thumbnail) =
                                stream_data.get("thumbnail_url").and_then(|v| v.as_str())
                            {
                                stream.thumbnail_url = thumbnail.to_string();
                            }

                            // Update started_at if available
                            if let Some(started_at) =
                                stream_data.get("started_at").and_then(|v| v.as_str())
                            {
                                stream.started_at = started_at.to_string();
                            }
                        }
                    }
                }

                // Append exact match if query looks like a username, to patch Twitch's search algo filtering out inactive direct matches
                let trimmed_query = query.trim();
                if !trimmed_query.contains(' ')
                    && !streams
                        .iter()
                        .any(|s| s.user_login.eq_ignore_ascii_case(trimmed_query))
                {
                    if let Ok(exact_user) = Self::get_user_by_login(trimmed_query).await {
                        let synthesize = TwitchStream {
                            id: exact_user.id.clone(),
                            user_id: exact_user.id,
                            user_name: exact_user.display_name,
                            user_login: exact_user.login,
                            title: String::new(),
                            viewer_count: 0,
                            game_id: String::new(),
                            game_name: String::new(),
                            thumbnail_url: exact_user.profile_image_url.clone().unwrap_or_default(),
                            started_at: String::new(),
                            broadcaster_type: exact_user.broadcaster_type,
                            has_shared_chat: None,
                            profile_image_url: exact_user.profile_image_url,
                            is_live: Some(false),
                            tags: None,
                            language: None,
                        };
                        streams.insert(0, synthesize);
                    }
                }

                Ok(streams)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Follow a channel via Twitch GQL persisted mutation (FollowButton_FollowUser).
    /// Hash captured live from Twitch web client on 2025-03-25.
    ///
    /// Uses the Android-app client id paired with the device-login OAuth token from
    /// DropsAuthService. Twitch does not gate Android-client mutations behind
    /// Client-Integrity, so no browser-minted integrity token (and no hidden
    /// webview) is involved. The OAuth token and the Client-Id must be issued for
    /// the same app, which is why both come from the Android pair here.
    pub async fn follow_channel(target_user_id: &str) -> Result<()> {
        use crate::services::drops_auth_service::DropsAuthService;

        let token = DropsAuthService::get_token().await?;
        let client = crate::services::http::client().clone();

        const GQL_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let payload = serde_json::json!({
            "operationName": "FollowButton_FollowUser",
            "variables": {
                "input": {
                    "disableNotifications": false,
                    "targetID": target_user_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "800e7346bdf7e5278a3c1d3f21b2b56e2639928f86815677a7126b093b2fdd08"
                }
            }
        });

        debug!(
            "[follow_channel] Sending GQL FollowButton_FollowUser for target: {}",
            target_user_id
        );

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", GQL_CLIENT_ID)
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header(ACCEPT, "*/*")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GQL follow failed (HTTP {}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        // Check for GQL-level errors
        if let Some(errors) = body.get("errors") {
            return Err(anyhow::anyhow!("GQL follow errors: {:?}", errors));
        }

        debug!(
            "[follow_channel] Successfully followed user {}",
            target_user_id
        );
        Ok(())
    }

    /// Unfollow a channel via Twitch GQL persisted mutation (FollowButton_UnfollowUser).
    /// Hash captured live from Twitch web client on 2025-03-25.
    ///
    /// Same Android-client credential as `follow_channel`: the DropsAuthService
    /// device-login token plus the Android Client-Id, integrity-free.
    pub async fn unfollow_channel(target_user_id: &str) -> Result<()> {
        use crate::services::drops_auth_service::DropsAuthService;

        let token = DropsAuthService::get_token().await?;
        let client = crate::services::http::client().clone();

        const GQL_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let payload = serde_json::json!({
            "operationName": "FollowButton_UnfollowUser",
            "variables": {
                "input": {
                    "targetID": target_user_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "f7dae976ebf41c755ae2d758546bfd176b4eeb856656098bb40e0a672ca0d880"
                }
            }
        });

        debug!(
            "[unfollow_channel] Sending GQL FollowButton_UnfollowUser for target: {}",
            target_user_id
        );

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", GQL_CLIENT_ID)
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header(ACCEPT, "*/*")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GQL unfollow failed (HTTP {}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        if let Some(errors) = body.get("errors") {
            return Err(anyhow::anyhow!("GQL unfollow errors: {:?}", errors));
        }

        debug!(
            "[unfollow_channel] Successfully unfollowed user {}",
            target_user_id
        );
        Ok(())
    }

    /// Fetch pinned chat messages for a channel via Twitch GQL (GetPinnedChat)
    /// Hash captured live from Twitch web client on 2026-03-25
    pub async fn get_pinned_chat_messages(channel_id: &str) -> Result<Vec<serde_json::Value>> {
        use crate::services::drops_auth_service::DropsAuthService;

        let token = DropsAuthService::get_token().await?;
        let client = crate::services::http::client().clone();

        const GQL_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let payload = serde_json::json!({
            "operationName": "GetPinnedChat",
            "variables": {
                "channelID": channel_id,
                "count": 10
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "2d099d4c9b6af80a07d8440140c4f3dbb04d516b35c401aab7ce8f60765308d5"
                }
            }
        });

        debug!(
            "[get_pinned_chat] Fetching pinned messages for channel: {}",
            channel_id
        );

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", GQL_CLIENT_ID)
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header(ACCEPT, "*/*")
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "GQL GetPinnedChat failed (HTTP {}): {}",
                status,
                error_text
            ));
        }

        let body: serde_json::Value = response.json().await?;

        if let Some(errors) = body.get("errors") {
            return Err(anyhow::anyhow!("GQL GetPinnedChat errors: {:?}", errors));
        }

        // Parse pinned messages and collect unique user IDs for avatar lookup
        let mut raw_pins = Vec::new();
        let mut user_ids = std::collections::HashSet::new();

        if let Some(edges) = body
            .pointer("/data/channel/pinnedChatMessages/edges")
            .and_then(|v| v.as_array())
        {
            for edge in edges {
                if let Some(node) = edge.get("node") {
                    let sender_id = node
                        .pointer("/pinnedMessage/sender/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let pinned_by_id = node
                        .pointer("/pinnedBy/id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    if !sender_id.is_empty() {
                        user_ids.insert(sender_id.to_string());
                    }
                    if !pinned_by_id.is_empty() {
                        user_ids.insert(pinned_by_id.to_string());
                    }

                    // Extract badges array
                    let badges: Vec<serde_json::Value> = node
                        .pointer("/pinnedMessage/sender/displayBadges")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter().map(|b| {
                            serde_json::json!({
                                "set_id": b.get("setID").and_then(|v| v.as_str()).unwrap_or(""),
                                "version": b.get("version").and_then(|v| v.as_str()).unwrap_or("1"),
                            })
                        }).collect()
                        })
                        .unwrap_or_default();

                    raw_pins.push((
                        node.clone(),
                        sender_id.to_string(),
                        pinned_by_id.to_string(),
                        badges,
                    ));
                }
            }
        }

        // Batch fetch profile images via Helix API for all unique user IDs
        let mut avatar_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if !user_ids.is_empty() {
            if let Ok(helix_token) = Self::get_token().await {
                let ids: Vec<&str> = user_ids.iter().map(|s| s.as_str()).collect();
                let mut url = String::from("https://api.twitch.tv/helix/users?");
                for (i, id) in ids.iter().enumerate() {
                    if i > 0 {
                        url.push('&');
                    }
                    url.push_str(&format!("id={}", id));
                }

                if let Ok(resp) = client
                    .get(&url)
                    .header("Client-ID", CLIENT_ID)
                    .header(AUTHORIZATION, format!("Bearer {}", helix_token))
                    .send()
                    .await
                {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(users) = data.get("data").and_then(|v| v.as_array()) {
                            for user in users {
                                if let (Some(id), Some(img)) = (
                                    user.get("id").and_then(|v| v.as_str()),
                                    user.get("profile_image_url").and_then(|v| v.as_str()),
                                ) {
                                    avatar_map.insert(id.to_string(), img.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build final response with avatars
        let pinned_messages: Vec<serde_json::Value> = raw_pins.iter().map(|(node, sender_id, pinned_by_id, badges)| {
            serde_json::json!({
                "id": node.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                // The underlying chat message id (NOT the pin-record id above), so the
                // client can tell which live chat message is currently pinned.
                "message_id": node.pointer("/pinnedMessage/id").and_then(|v| v.as_str()).unwrap_or(""),
                "type": node.get("type").and_then(|v| v.as_str()).unwrap_or("MOD"),
                "message_text": node.pointer("/pinnedMessage/content/text").and_then(|v| v.as_str()).unwrap_or(""),
                "sender_id": sender_id,
                "sender_name": node.pointer("/pinnedMessage/sender/displayName").and_then(|v| v.as_str()).unwrap_or("Unknown"),
                "sender_color": node.pointer("/pinnedMessage/sender/chatColor").and_then(|v| v.as_str()).unwrap_or("#FFFFFF"),
                "sender_avatar": avatar_map.get(sender_id).cloned().unwrap_or_default(),
                "sender_badges": badges,
                "pinned_by": node.pointer("/pinnedBy/displayName").and_then(|v| v.as_str()).unwrap_or(""),
                "pinned_by_id": pinned_by_id,
                "pinned_by_avatar": avatar_map.get(pinned_by_id).cloned().unwrap_or_default(),
                "started_at": node.get("startsAt").and_then(|v| v.as_str()).unwrap_or(""),
            })
        }).collect();

        debug!(
            "[get_pinned_chat] Found {} pinned messages for channel {}",
            pinned_messages.len(),
            channel_id
        );
        Ok(pinned_messages)
    }

    pub async fn get_all_followed_channels(
        limit: u32,
        cursor: Option<String>,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let user_info = Self::get_user_info().await?;

        let mut url = format!(
            "https://api.twitch.tv/helix/channels/followed?user_id={}&first={}",
            user_info.id, limit
        );
        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let next_cursor = response
            .get("pagination")
            .and_then(|p| p.get("cursor"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams = Vec::new();
                let mut user_ids = Vec::new();

                for channel in arr {
                    if let (
                        Some(broadcaster_id),
                        Some(broadcaster_login),
                        Some(broadcaster_name),
                        Some(followed_at),
                    ) = (
                        channel.get("broadcaster_id").and_then(|v| v.as_str()),
                        channel.get("broadcaster_login").and_then(|v| v.as_str()),
                        channel.get("broadcaster_name").and_then(|v| v.as_str()),
                        channel.get("followed_at").and_then(|v| v.as_str()),
                    ) {
                        user_ids.push(broadcaster_id.to_string());

                        streams.push(TwitchStream {
                            id: broadcaster_id.to_string(),
                            user_id: broadcaster_id.to_string(),
                            user_name: broadcaster_name.to_string(),
                            user_login: broadcaster_login.to_string(),
                            title: String::new(),
                            viewer_count: 0,
                            game_id: String::new(),
                            game_name: String::new(),
                            thumbnail_url: String::new(),
                            started_at: followed_at.to_string(), // use started_at to store followed_at temporarily
                            broadcaster_type: None,
                            has_shared_chat: None,
                            profile_image_url: None,
                            is_live: Some(false),
                            tags: None,
                            language: None,
                        });
                    }
                }

                // Fetch actual user avatars via helix/users
                if !user_ids.is_empty() {
                    // Split into chunks of 100 (Twitch API max) just in case
                    for chunk in user_ids.chunks(100) {
                        let user_ids_param = chunk
                            .iter()
                            .map(|id| format!("id={}", id))
                            .collect::<Vec<_>>()
                            .join("&");

                        let users_url =
                            format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                        if let Ok(users_response) = client
                            .get(&users_url)
                            .header("Client-Id", CLIENT_ID)
                            .header(AUTHORIZATION, format!("Bearer {}", token))
                            .send()
                            .await
                        {
                            if let Ok(users_json) = users_response.json::<serde_json::Value>().await
                            {
                                if let Some(users_data) =
                                    users_json.get("data").and_then(|d| d.as_array())
                                {
                                    let mut profiles = std::collections::HashMap::new();
                                    for user in users_data {
                                        if let (Some(id), Some(profile_image_url)) = (
                                            user.get("id").and_then(|v| v.as_str()),
                                            user.get("profile_image_url").and_then(|v| v.as_str()),
                                        ) {
                                            profiles.insert(
                                                id.to_string(),
                                                profile_image_url.to_string(),
                                            );
                                        }
                                    }

                                    for stream in &mut streams {
                                        if let Some(avatar) = profiles.get(&stream.user_id) {
                                            stream.profile_image_url = Some(avatar.clone());
                                            stream.thumbnail_url = avatar.clone();
                                            // use avatar as thumbnail fallback
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Ok((streams, next_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_offline_last_broadcasts(
        user_ids: Vec<String>,
    ) -> Result<std::collections::HashMap<String, Option<String>>> {
        let client = crate::services::http::client().clone();
        // create the query
        let query = format!(
            r#"query {{ users(ids: {}) {{ id lastBroadcast {{ startedAt }} videos(first: 1, sort: TIME) {{ edges {{ node {{ createdAt lengthSeconds }} }} }} }} }}"#,
            serde_json::to_string(&user_ids).unwrap_or_else(|_| "[]".to_string())
        );

        let payload = serde_json::json!([{
            "operationName": null,
            "variables": {},
            "query": query
        }]);

        // IMPORTANT: Unauthenticated GQL requires the Twitch Web Client-ID, NOT the Helix Developer application Client-ID
        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
            .json(&payload)
            .send()
            .await?;

        let text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)?;

        let mut result_map = std::collections::HashMap::new();

        if let Some(arr) = json.as_array() {
            if let Some(users_data) = arr
                .first()
                .and_then(|obj| obj.get("data"))
                .and_then(|d| d.get("users"))
                .and_then(|u| u.as_array())
            {
                for user in users_data {
                    if let Some(id) = user.get("id").and_then(|i| i.as_str()) {
                        let mut final_time = None;

                        // Try calculating exact true end time using their most recent VOD/Stream duration
                        if let Some(edges) = user
                            .get("videos")
                            .and_then(|v| v.get("edges"))
                            .and_then(|e| e.as_array())
                        {
                            if let Some(node) = edges.first().and_then(|e| e.get("node")) {
                                if let (Some(created_at), Some(length)) = (
                                    node.get("createdAt").and_then(|c| c.as_str()),
                                    node.get("lengthSeconds").and_then(|l| l.as_i64()),
                                ) {
                                    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(created_at)
                                    {
                                        let end_time = dt + ChronoDuration::seconds(length);
                                        // Format back to RFC3339 string so frontend can trivially parse it using new Date()
                                        final_time = Some(end_time.to_rfc3339());
                                    }
                                }
                            }
                        }

                        // Fallback to start time for streamers who completely disable all VOD recordings
                        if final_time.is_none() {
                            final_time = user
                                .get("lastBroadcast")
                                .and_then(|lb| lb.get("startedAt"))
                                .and_then(|sa| sa.as_str())
                                .map(|s| s.to_string());
                        }

                        result_map.insert(id.to_string(), final_time);
                    }
                }
            }
        }

        debug!(
            "[TWITCH] Parsed {} last broadcast timestamps",
            result_map.len()
        );
        Ok(result_map)
    }

    pub async fn check_following_status(target_user_id: &str) -> Result<bool> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // Get the current user's ID
        let user_info = Self::get_user_info().await?;

        // Use the new channels/followed endpoint (the old users/follows was deprecated)
        // This endpoint checks if user_id follows broadcaster_id
        let url = format!(
            "https://api.twitch.tv/helix/channels/followed?user_id={}&broadcaster_id={}",
            user_info.id, target_user_id
        );

        let response = client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .send()
            .await?;

        // Check if request was successful
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            debug!(
                "[check_following_status] API error: {} - {}",
                status, error_text
            );
            // Return false if we can't check (might not be following)
            return Ok(false);
        }

        let json: serde_json::Value = response.json().await?;

        // If data array has items, user is following
        // The new endpoint returns total in the response and data array with the follow info
        let is_following = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);

        debug!(
            "[check_following_status] User {} following {}: {}",
            user_info.id, target_user_id, is_following
        );

        Ok(is_following)
    }

    /// Check if a specific stream is currently online by user login
    /// Returns the stream data if online, None if offline
    pub async fn check_stream_online(user_login: &str) -> Result<Option<TwitchStream>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/streams?user_login={}",
            urlencoding::encode(user_login)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) if !arr.is_empty() => {
                let stream: TwitchStream = serde_json::from_value(arr[0].clone())?;
                Ok(Some(stream))
            }
            _ => Ok(None), // Stream is offline
        }
    }

    /// Batched liveness check. Helix `streams` accepts up to 100 `user_login`
    /// params per call, so an N-login allow-list resolves in ceil(N/100) requests
    /// instead of N. Offline logins are simply absent from the response, so the
    /// returned vec is exactly the live subset. This replaces firing one
    /// `check_stream_online` per channel (an ACL like EWC has ~1180 channels; the
    /// per-channel fan-out rate-limited and silently dropped live channels).
    pub async fn check_streams_online(user_logins: &[String]) -> Result<Vec<TwitchStream>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut live: Vec<TwitchStream> = Vec::new();

        for chunk in user_logins.chunks(100) {
            // ?first=100&user_login=a&user_login=b... — first=100 so a fully-live
            // chunk isn't truncated by Helix's default page size of 20.
            let params: String = chunk
                .iter()
                .map(|l| format!("user_login={}", urlencoding::encode(l)))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/streams?first=100&{}", params);

            let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
            if let Some(token) = &token {
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }

            let response = request.send().await?.json::<serde_json::Value>().await?;
            if let Some(arr) = response.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    if let Ok(stream) = serde_json::from_value::<TwitchStream>(item.clone()) {
                        live.push(stream);
                    }
                }
            }
        }

        Ok(live)
    }

    /// Get the game ID by game name
    pub async fn get_game_id_by_name(game_name: &str) -> Result<Option<String>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/games?name={}",
            urlencoding::encode(game_name)
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let game_id = response
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|game| game.get("id"))
            .and_then(|id| id.as_str())
            .map(|s| s.to_string());

        Ok(game_id)
    }

    /// Resolve category ids → their objects (`{id, name, box_art_url}`) via Helix
    /// Get Games (up to 100 ids per call). Used to label a streamer's clips with
    /// the category they were taken in (clips carry `game_id`, not the name).
    pub async fn get_games_by_ids(ids: &[String]) -> Result<Vec<serde_json::Value>> {
        let query: Vec<String> = ids
            .iter()
            .filter(|id| !id.is_empty())
            .take(100)
            .map(|id| format!("id={}", urlencoding::encode(id)))
            .collect();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();
        let url = format!("https://api.twitch.tv/helix/games?{}", query.join("&"));

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }
        let response = request.send().await?.json::<serde_json::Value>().await?;

        Ok(response
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// Search for categories by name (uses Twitch search API for fuzzy matching)
    /// Returns a list of matching categories with id, name, and box_art_url
    pub async fn search_categories(query: &str, limit: u32) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/search/categories?query={}&first={}",
            urlencoding::encode(query),
            limit
        );

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let data = response
            .get("data")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(data)
    }

    /// Look up categories by EXACT name, many at a time.
    ///
    /// `search/categories` is a fuzzy search, so it answers "something like
    /// this" and cannot tell a caller whether a given string is genuinely a
    /// category. This one can: Helix `games?name=` matches exactly and takes up
    /// to 100 names per request, which is what makes it affordable to test a
    /// whole list of candidate names at once. Names Twitch doesn't know are
    /// simply absent from the response.
    pub async fn categories_by_name(names: &[String]) -> Result<Vec<serde_json::Value>> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut found = Vec::new();
        for chunk in names.chunks(100) {
            let query = chunk
                .iter()
                .map(|n| format!("name={}", urlencoding::encode(n)))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/games?{}", query);

            let mut request = client.get(&url).header("Client-Id", CLIENT_ID);
            if let Some(token) = &token {
                request = request.header(AUTHORIZATION, format!("Bearer {}", token));
            }

            let response = request.send().await?.json::<serde_json::Value>().await?;
            if let Some(data) = response.get("data").and_then(|d| d.as_array()) {
                found.extend(data.iter().cloned());
            }
        }

        Ok(found)
    }

    /// Send a whisper message to another user
    /// Requires user:manage:whispers scope
    pub async fn send_whisper(to_user_id: &str, message: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // Get the current user's ID
        let user_info = Self::get_user_info().await?;

        let response = client
            .post(format!(
                "https://api.twitch.tv/helix/whispers?from_user_id={}&to_user_id={}",
                user_info.id, to_user_id
            ))
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Client-Id", CLIENT_ID)
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "message": message
            }))
            .send()
            .await?;

        if response.status().is_success() || response.status() == 204 {
            Ok(())
        } else {
            let error_text = response.text().await?;
            Err(anyhow::anyhow!("Failed to send whisper: {}", error_text))
        }
    }

    /// Get streams by game name (convenience method that resolves game name to ID)
    /// Returns streams sorted by viewer count (highest first)
    pub async fn get_streams_by_game_name(
        state: &AppState,
        game_name: &str,
        exclude_user_login: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        // Resolve the display name to a category id, then reuse the id-based path.
        let game_id = match Self::get_game_id_by_name(game_name).await? {
            Some(id) => id,
            None => return Ok((Vec::new(), None)), // Game not found
        };

        Self::get_streams_by_game_id(state, &game_id, exclude_user_login, cursor, limit).await
    }

    /// Fetch live streams in a category by its id, skipping the name→id lookup.
    /// Prefer this when the caller already holds the category id (e.g. the live
    /// `game_id` carried on the stream being watched).
    pub async fn get_streams_by_game_id(
        _state: &AppState,
        game_id: &str,
        exclude_user_login: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let token = Self::get_token().await.ok();
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/streams?game_id={}&first={}",
            game_id, limit
        );

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let mut request = client.get(&url).header("Client-Id", CLIENT_ID);

        if let Some(token) = &token {
            request = request.header(AUTHORIZATION, format!("Bearer {}", token));
        }

        let response = request.send().await?.json::<serde_json::Value>().await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut streams: Vec<TwitchStream> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                // Filter out the excluded user if provided
                if let Some(exclude_login) = exclude_user_login {
                    streams.retain(|s| s.user_login.to_lowercase() != exclude_login.to_lowercase());
                }

                // Fetch broadcaster types for all streams in a batch
                if !streams.is_empty() && token.is_some() {
                    let user_ids: Vec<String> = streams.iter().map(|s| s.user_id.clone()).collect();
                    let user_ids_param = user_ids
                        .iter()
                        .map(|id| format!("id={}", id))
                        .collect::<Vec<_>>()
                        .join("&");

                    let users_url = format!("https://api.twitch.tv/helix/users?{}", user_ids_param);
                    let users_response = client
                        .get(&users_url)
                        .header("Client-Id", CLIENT_ID)
                        .header(AUTHORIZATION, format!("Bearer {}", token.unwrap()))
                        .send()
                        .await?
                        .json::<serde_json::Value>()
                        .await?;

                    if let Some(users_data) = users_response.get("data").and_then(|d| d.as_array())
                    {
                        let mut broadcaster_types = std::collections::HashMap::new();
                        let mut profile_images = std::collections::HashMap::new();
                        for user in users_data {
                            if let Some(id) = user.get("id").and_then(|v| v.as_str()) {
                                if let Some(broadcaster_type) =
                                    user.get("broadcaster_type").and_then(|v| v.as_str())
                                {
                                    if !broadcaster_type.is_empty() {
                                        broadcaster_types
                                            .insert(id.to_string(), broadcaster_type.to_string());
                                    }
                                }
                                if let Some(profile_image) =
                                    user.get("profile_image_url").and_then(|v| v.as_str())
                                {
                                    profile_images
                                        .insert(id.to_string(), profile_image.to_string());
                                }
                            }
                        }

                        for stream in &mut streams {
                            if let Some(broadcaster_type) = broadcaster_types.get(&stream.user_id) {
                                stream.broadcaster_type = Some(broadcaster_type.clone());
                            }
                            if let Some(profile_image) = profile_images.get(&stream.user_id) {
                                stream.profile_image_url = Some(profile_image.clone());
                            }
                        }
                    }
                }

                // Extract pagination cursor
                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                // Streams are already sorted by viewer count (highest first) from the API
                Ok((streams, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    pub async fn get_category_info(
        game_name: &str,
    ) -> Result<Option<crate::models::stream::CategoryInfo>> {
        let client = crate::services::http::client().clone();
        let query = "query($name: String!) { game(name: $name) { id name displayName description followersCount boxArtURL tags(tagType: CONTENT) { id localizedName } } }";

        let body = serde_json::json!({
            "query": query,
            "variables": { "name": game_name }
        });

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", TWITCH_GQL_CLIENT_ID)
            .json(&body)
            .send()
            .await?;

        if response.status().is_success() {
            let data: crate::models::stream::GqlDataResponse = response.json().await?;
            if let Some(game_data) = data.data {
                return Ok(game_data.game);
            }
        }

        Ok(None)
    }

    /// Live streams in a category filtered by freeform tags, server-side, via
    /// GQL. Helix can't filter streams by tag, so this is the only way to match
    /// Twitch's directory tag filter (returns every matching stream by viewer
    /// count, not just whatever happens to be on the loaded Helix pages). `tags`
    /// are tag display names (e.g. "Speedrun"); a stream must carry all of them.
    pub async fn get_streams_by_game_with_tags(
        game_name: &str,
        tags: Vec<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<(Vec<TwitchStream>, Option<String>)> {
        let client = crate::services::http::client().clone();

        // `options.freeformTags` is the field that actually filters by freeform
        // tag NAME (case-insensitive), server-side, ranked by viewer count — this
        // is Twitch's directory tag filter. The plain `tags` argument is silently
        // ignored (legacy), so it must NOT be used. Partner status comes from
        // `roles` (GQL `User` has no `broadcasterType`).
        let query = "query($name: String!, $first: Int!, $after: Cursor, $tags: [String!]) { \
            game(name: $name) { id name \
                streams(first: $first, after: $after, options: { sort: VIEWER_COUNT, freeformTags: $tags }) { \
                    edges { cursor node { \
                        id title viewersCount createdAt type language \
                        previewImageURL \
                        freeformTags { name } \
                        broadcaster { id login displayName profileImageURL(width: 70) roles { isPartner isAffiliate } } \
                    } } \
                    pageInfo { hasNextPage } \
                } \
            } }";

        let body = serde_json::json!({
            "query": query,
            "variables": {
                "name": game_name,
                "first": limit,
                "after": cursor,
                "tags": tags,
            }
        });

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", TWITCH_GQL_CLIENT_ID)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let game_id = response
            .pointer("/data/game/id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let resolved_game_name = response
            .pointer("/data/game/name")
            .and_then(|v| v.as_str())
            .unwrap_or(game_name)
            .to_string();

        let streams_node = response.pointer("/data/game/streams");
        let edges = streams_node
            .and_then(|s| s.get("edges"))
            .and_then(|e| e.as_array());

        let mut streams: Vec<TwitchStream> = Vec::new();
        let mut last_cursor: Option<String> = None;

        if let Some(edges) = edges {
            for edge in edges {
                if let Some(c) = edge.get("cursor").and_then(|c| c.as_str()) {
                    last_cursor = Some(c.to_string());
                }
                let node = match edge.get("node") {
                    Some(n) if !n.is_null() => n,
                    _ => continue,
                };
                let broadcaster = match node.get("broadcaster") {
                    Some(b) if !b.is_null() => b,
                    _ => continue,
                };

                let user_login = broadcaster
                    .get("login")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let user_name = broadcaster
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&user_login)
                    .to_string();
                let roles = broadcaster.get("roles");
                let broadcaster_type = if roles
                    .and_then(|r| r.get("isPartner"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("partner".to_string())
                } else if roles
                    .and_then(|r| r.get("isAffiliate"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    Some("affiliate".to_string())
                } else {
                    None
                };
                let profile_image_url = broadcaster
                    .get("profileImageURL")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let stream_tags: Vec<String> = node
                    .get("freeformTags")
                    .and_then(|t| t.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| {
                                t.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                streams.push(TwitchStream {
                    id: node.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_id: broadcaster.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    user_name,
                    user_login,
                    title: node.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    viewer_count: node
                        .get("viewersCount")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32,
                    game_id: game_id.clone(),
                    game_name: resolved_game_name.clone(),
                    thumbnail_url: node
                        .get("previewImageURL")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    started_at: node.get("createdAt").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    broadcaster_type,
                    has_shared_chat: None,
                    profile_image_url,
                    is_live: Some(true),
                    tags: if stream_tags.is_empty() { None } else { Some(stream_tags) },
                    language: node
                        .get("language")
                        .and_then(|v| v.as_str())
                        .map(normalize_gql_language),
                });
            }
        }

        let has_next = streams_node
            .and_then(|s| s.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|h| h.as_bool())
            .unwrap_or(false);

        Ok((streams, if has_next { last_cursor } else { None }))
    }

    pub async fn get_clips_by_game(
        game_id: &str,
        limit: u32,
        cursor: Option<&str>,
        period: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchClip>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/clips?game_id={}&first={}",
            game_id, limit
        );

        if let Some(p) = period {
            if p != "all" && !p.is_empty() {
                let now = chrono::Utc::now();
                let started_at = match p {
                    "day" | "24h" => Some(now - chrono::Duration::days(1)),
                    "week" | "7d" => Some(now - chrono::Duration::days(7)),
                    "month" | "30d" => Some(now - chrono::Duration::days(30)),
                    _ => None,
                };

                if let Some(date) = started_at {
                    // Helix Get Clips needs BOTH bounds for the time filter to
                    // apply — with only started_at it ignores the window (every
                    // period returns the same set). RFC3339 required.
                    url.push_str(&format!(
                        "&started_at={}&ended_at={}",
                        date.to_rfc3339(),
                        now.to_rfc3339()
                    ));
                }
            }
        }

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let clips: Vec<crate::models::stream::TwitchClip> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;

                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                Ok((clips, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    /// POST an inline GQL read query for public channel lists (clips, videos)
    /// with per-call device + session ids and no OAuth. Sends the Android client
    /// id, not the web client id: Twitch now fails the integrity check on
    /// paginated anonymous web-client reads, so the first page loads but "load
    /// more" comes back with "failed integrity check". The Android client id is
    /// exempt from that challenge (the same reason follows and reactions route
    /// through it), so pagination works without minting a Client-Integrity token.
    /// The device/session ids keep us out of the harsh anonymous rate-limit
    /// bucket.
    async fn gql_public_read(body: serde_json::Value) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let device_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        let session_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        let resp = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
            .header(ACCEPT, "*/*")
            .header("X-Device-Id", device_id)
            .header("Client-Session-Id", session_id)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Surface top-level GraphQL errors (bad field, throttling, etc.) instead
        // of silently returning an empty list.
        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let msg = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(anyhow::anyhow!("Twitch GQL error: {}", msg));
            }
        }

        Ok(resp)
    }

    /// One streamer's clips, fetched the way the Twitch website's Clips tab does:
    /// `user.clips(criteria: { filter: <period> })`. Unlike Helix `/clips`, this
    /// returns the correct time window (Helix caps an open-ended `started_at` at
    /// one week, breaking the 30-day filter) AND orders by view count (Helix has
    /// no sort), so "Top clips (last 7 days)" etc. match twitch.tv exactly. Game
    /// names come back inline, so no follow-up `get_games_by_ids` round-trip.
    pub async fn get_clips_by_broadcaster(
        broadcaster_id: &str,
        limit: u32,
        cursor: Option<&str>,
        period: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchClip>, Option<String>)> {
        // Map our period strings → Twitch's ClipsPeriod enum (always has a value;
        // "all" → ALL_TIME). The value is a fixed keyword, never user input, so
        // interpolating it into the query is safe.
        let filter = match period.unwrap_or("all") {
            "24h" | "day" => "LAST_DAY",
            "7d" | "week" => "LAST_WEEK",
            "30d" | "month" => "LAST_MONTH",
            _ => "ALL_TIME",
        };

        let query = format!(
            r#"query StreamNookUserClips($id: ID!, $first: Int!, $after: Cursor) {{
                user(id: $id) {{
                    clips(first: $first, after: $after, criteria: {{ filter: {filter} }}) {{
                        edges {{
                            cursor
                            node {{
                                slug
                                url
                                embedURL
                                title
                                viewCount
                                createdAt
                                durationSeconds
                                thumbnailURL
                                videoOffsetSeconds
                                video {{ id }}
                                game {{ id name }}
                                curator {{ id displayName }}
                                broadcaster {{ id displayName login }}
                            }}
                        }}
                        pageInfo {{ hasNextPage }}
                    }}
                }}
            }}"#,
            filter = filter
        );

        let body = serde_json::json!({
            "operationName": "StreamNookUserClips",
            "query": query,
            "variables": { "id": broadcaster_id, "first": limit, "after": cursor },
        });

        let resp = Self::gql_public_read(body).await?;
        let clips_node = resp
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("clips"));

        let edges = clips_node
            .and_then(|c| c.get("edges"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let str_at = |v: &serde_json::Value, path: &[&str]| -> String {
            let mut cur = v;
            for k in path {
                match cur.get(k) {
                    Some(next) => cur = next,
                    None => return String::new(),
                }
            }
            cur.as_str().unwrap_or("").to_string()
        };

        let mut clips = Vec::with_capacity(edges.len());
        let mut last_cursor: Option<String> = None;
        for edge in &edges {
            last_cursor = edge
                .get("cursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let node = match edge.get("node") {
                Some(n) if !n.is_null() => n,
                _ => continue,
            };
            let slug = str_at(node, &["slug"]);
            let game_name = node
                .get("game")
                .and_then(|g| g.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            clips.push(crate::models::stream::TwitchClip {
                id: slug.clone(),
                url: str_at(node, &["url"]),
                embed_url: str_at(node, &["embedURL"]),
                broadcaster_id: str_at(node, &["broadcaster", "id"]),
                broadcaster_name: str_at(node, &["broadcaster", "displayName"]),
                broadcaster_login: node
                    .get("broadcaster")
                    .and_then(|b| b.get("login"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_lowercase()),
                creator_id: str_at(node, &["curator", "id"]),
                creator_name: str_at(node, &["curator", "displayName"]),
                video_id: str_at(node, &["video", "id"]),
                game_id: str_at(node, &["game", "id"]),
                game_name,
                language: String::new(),
                title: str_at(node, &["title"]),
                view_count: node.get("viewCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                created_at: str_at(node, &["createdAt"]),
                thumbnail_url: str_at(node, &["thumbnailURL"]),
                duration: node
                    .get("durationSeconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32,
                vod_offset: node
                    .get("videoOffsetSeconds")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32),
            });
        }

        let has_next = clips_node
            .and_then(|c| c.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let next = if has_next { last_cursor } else { None };

        Ok((clips, next))
    }

    /// Reaction ("likes") counts for a batch of clips, via the
    /// `aggregateReactionsBreakdownByContentKeys` query (NOT on the clip list
    /// type — Twitch loads it per-clip). Mirrors the app's proven authed
    /// persisted-hash path (Android client id + drops OAuth token, integrity-free;
    /// the app token + web client id 401s). Twitch fails the WHOLE request if any
    /// single content key is invalid (e.g. an old clip that predates reactions),
    /// so we chunk and, on that error, retry the chunk one clip at a time — every
    /// valid clip still gets its count. Hash captured 2026-06-03.
    pub async fn get_clip_reactions(
        slugs: &[String],
    ) -> Result<Vec<crate::models::stream::ClipReactions>> {
        if slugs.is_empty() {
            return Ok(Vec::new());
        }
        // Auth once (Android client id + drops token; see fetch_reactions_batch).
        let token = crate::services::drops_auth_service::DropsAuthService::get_token().await?;

        let mut out: Vec<crate::models::stream::ClipReactions> = Vec::new();
        for chunk in slugs.chunks(20) {
            match Self::fetch_reactions_batch(chunk, &token).await {
                Ok(rows) => out.extend(rows),
                // One clip in the chunk is un-reactable and Twitch rejected the
                // entire batch; retry each clip alone, skipping the bad one(s).
                Err(e) if e.to_string().to_lowercase().contains("content key") => {
                    for s in chunk {
                        if let Ok(rows) =
                            Self::fetch_reactions_batch(std::slice::from_ref(s), &token).await
                        {
                            out.extend(rows);
                        }
                    }
                }
                // Auth/transport failure — fatal for every chunk, so surface it.
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// One reactions request for a batch of clip slugs (see get_clip_reactions).
    /// Errs with the GraphQL/auth message; the caller keys off "content key" to
    /// decide whether to split-and-retry.
    async fn fetch_reactions_batch(
        slugs: &[String],
        token: &str,
    ) -> Result<Vec<crate::models::stream::ClipReactions>> {
        let client = crate::services::http::client().clone();
        let device_id = uuid::Uuid::new_v4().to_string().replace('-', "");
        let session_id = uuid::Uuid::new_v4().to_string().replace('-', "");

        let content_keys: Vec<serde_json::Value> = slugs
            .iter()
            .map(|s| serde_json::json!({ "id": s, "type": "CLIP" }))
            .collect();

        let body = serde_json::json!({
            "operationName": "aggregateReactionsBreakdownByContentKeys",
            "variables": { "contentKeys": content_keys },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": "f74807b6d62e390c87a2dc140ee24cc6e6c989c5e5170f9ebe32a39014da86fd"
                }
            }
        });

        let resp = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-ID", env!("TWITCH_ANDROID_CLIENT_ID"))
            .header(ACCEPT, "*/*")
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .header("Origin", "https://www.twitch.tv")
            .header("Referer", "https://www.twitch.tv")
            .header("X-Device-Id", device_id)
            .header("Client-Session-Id", session_id)
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        // Top-level HTTP-style error (e.g. 401 `{"error":"Unauthorized", ...}`) —
        // this is NOT a GraphQL `errors` array, so check it explicitly or it would
        // slip through as "empty totals".
        if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
            let msg = resp.get("message").and_then(|m| m.as_str()).unwrap_or(err);
            return Err(anyhow::anyhow!("Twitch reactions auth error: {}", msg));
        }

        if let Some(errors) = resp.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                let msg = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(anyhow::anyhow!("Twitch reactions error: {}", msg));
            }
        }

        let totals = resp
            .get("data")
            .and_then(|d| d.get("aggregateReactionsBreakdownByContentKeys"))
            .and_then(|a| a.get("totals"))
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();

        let mut out = Vec::with_capacity(totals.len());
        for t in &totals {
            let id = t
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                continue;
            }
            let total = t.get("totalCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let like = t
                .get("totalBreakdown")
                .and_then(|b| b.as_array())
                .map(|arr| {
                    arr.iter()
                        .find(|e| {
                            e.get("reaction")
                                .and_then(|r| r.get("type"))
                                .and_then(|ty| ty.as_str())
                                == Some("LIKE")
                        })
                        .and_then(|e| e.get("count"))
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0) as u32
                })
                .unwrap_or(0);
            out.push(crate::models::stream::ClipReactions { id, total, like });
        }

        // Empty `out` is valid here — a clip with no reactions just won't appear
        // in totals (e.g. older clips that predate the reactions feature).
        Ok(out)
    }

    pub async fn get_videos_by_game(
        game_id: &str,
        sort: &str,
        period: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchVideo>, Option<String>)> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut url = format!(
            "https://api.twitch.tv/helix/videos?game_id={}&sort={}&first={}",
            game_id, sort, limit
        );

        if let Some(p) = period {
            if !p.is_empty() && p != "all" {
                url.push_str(&format!("&period={}", p));
            }
        }

        if let Some(c) = cursor {
            url.push_str(&format!("&after={}", c));
        }

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let data = response.get("data").and_then(|d| d.as_array());

        match data {
            Some(arr) => {
                let mut videos: Vec<crate::models::stream::TwitchVideo> =
                    serde_json::from_value(serde_json::Value::Array(arr.clone()))?;
                // Helix only formats the length ("3h21m4s"); recover the seconds
                // so the card's progress bar has a denominator.
                for v in videos.iter_mut() {
                    if v.length_seconds.is_none() {
                        v.length_seconds = Self::parse_duration_str(&v.duration);
                    }
                }

                let pagination_cursor = response
                    .get("pagination")
                    .and_then(|p| p.get("cursor"))
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string());

                Ok((videos, pagination_cursor))
            }
            None => Ok((Vec::new(), None)),
        }
    }

    /// One streamer's videos, fetched the way the Twitch website's Videos tab
    /// does: `user.videos(sort:, type:)`. Twitch has no time-window filter for a
    /// user's videos (Helix only accepts `period` with a game_id, never a
    /// user_id), so the website offers Sort (Recent/Popular) + Type (broadcast
    /// kind) instead — this mirrors that. `sort` is "time" or "views"; `kind`
    /// is "archive" | "highlight" | "upload" (None = all types).
    pub async fn get_user_videos(
        user_id: &str,
        sort: &str,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<(Vec<crate::models::stream::TwitchVideo>, Option<String>)> {
        let sort_enum = match sort.to_lowercase().as_str() {
            "views" | "view" | "popular" => "VIEWS",
            _ => "TIME",
        };
        let type_enum: Option<&str> = match kind {
            Some("archive") => Some("ARCHIVE"),
            Some("highlight") => Some("HIGHLIGHT"),
            Some("upload") => Some("UPLOAD"),
            _ => None, // all types
        };

        let query = r#"query StreamNookUserVideos($id: ID!, $first: Int!, $after: Cursor, $sort: VideoSort, $type: BroadcastType) {
            user(id: $id) {
                videos(first: $first, after: $after, sort: $sort, type: $type) {
                    edges {
                        cursor
                        node {
                            id
                            title
                            publishedAt
                            createdAt
                            lengthSeconds
                            status
                            viewCount
                            previewThumbnailURL(width: 440, height: 248)
                            broadcastType
                            game { displayName }
                            owner { id login displayName }
                        }
                    }
                    pageInfo { hasNextPage }
                }
            }
        }"#;

        let body = serde_json::json!({
            "operationName": "StreamNookUserVideos",
            "query": query,
            "variables": {
                "id": user_id,
                "first": limit,
                "after": cursor,
                "sort": sort_enum,
                "type": type_enum,
            },
        });

        let resp = Self::gql_public_read(body).await?;
        let videos_node = resp
            .get("data")
            .and_then(|d| d.get("user"))
            .and_then(|u| u.get("videos"));

        let edges = videos_node
            .and_then(|v| v.get("edges"))
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();

        let str_at = |v: &serde_json::Value, path: &[&str]| -> String {
            let mut cur = v;
            for k in path {
                match cur.get(k) {
                    Some(next) => cur = next,
                    None => return String::new(),
                }
            }
            cur.as_str().unwrap_or("").to_string()
        };

        let mut videos = Vec::with_capacity(edges.len());
        let mut last_cursor: Option<String> = None;
        for edge in &edges {
            last_cursor = edge
                .get("cursor")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string());
            let node = match edge.get("node") {
                Some(n) if !n.is_null() => n,
                _ => continue,
            };
            let id = str_at(node, &["id"]);
            let published_at = str_at(node, &["publishedAt"]);
            // GQL gives length in seconds; the UI badge expects a Helix-style
            // string ("3h21m4s"). Format it to match.
            let length_secs = node
                .get("lengthSeconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            // GQL broadcastType is uppercase (ARCHIVE/HIGHLIGHT/UPLOAD); the UI's
            // label map expects Helix-style lowercase.
            let video_type = node
                .get("broadcastType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            videos.push(crate::models::stream::TwitchVideo {
                url: format!("https://www.twitch.tv/videos/{}", id),
                user_id: str_at(node, &["owner", "id"]),
                user_login: str_at(node, &["owner", "login"]),
                user_name: str_at(node, &["owner", "displayName"]),
                title: str_at(node, &["title"]),
                description: String::new(),
                created_at: if published_at.is_empty() {
                    str_at(node, &["createdAt"])
                } else {
                    published_at.clone()
                },
                published_at,
                thumbnail_url: str_at(node, &["previewThumbnailURL"]),
                viewable: "public".to_string(),
                view_count: node.get("viewCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                language: String::new(),
                video_type,
                duration: Self::fmt_duration_secs(length_secs),
                // GQL Video.status is RECORDING while the broadcast is live.
                status: node
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase()),
                length_seconds: Some(length_secs.min(u32::MAX as u64) as u32),
                game_name: Some(str_at(node, &["game", "displayName"])).filter(|g| !g.is_empty()),
                progress: None,
                stream_id: None,
                id,
            });
        }

        let has_next = videos_node
            .and_then(|v| v.get("pageInfo"))
            .and_then(|p| p.get("hasNextPage"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let next = if has_next { last_cursor } else { None };

        Ok((videos, next))
    }

    /// Read many VODs by id in ONE request, using GraphQL aliases.
    ///
    /// Returns `(fresh metadata, ids Twitch no longer has)`. Probed live
    /// 2026-09-12: a deleted id resolves to `null` for just its own alias and
    /// the rest of the response is intact, so one call both refreshes the
    /// Continue Watching cards and tells us which stored entries to forget.
    ///
    /// Ids travel as `$iN: ID!` variables. They come from our own store and
    /// are numeric in practice, but a query built by string interpolation is
    /// not a thing worth shipping.
    pub async fn get_videos_by_ids(
        ids: &[String],
    ) -> Result<(
        Vec<crate::services::vod_progress_service::VodRefresh>,
        Vec<String>,
    )> {
        /// Aliases per request. Kept well under any practical query-size cap;
        /// callers currently send at most a couple of dozen.
        const CHUNK: usize = 50;

        let mut found = Vec::new();
        let mut gone = Vec::new();

        for chunk in ids.chunks(CHUNK) {
            let params = (0..chunk.len())
                .map(|i| format!("$i{i}: ID!"))
                .collect::<Vec<_>>()
                .join(", ");
            let selections = (0..chunk.len())
                .map(|i| {
                    format!(
                        "v{i}: video(id: $i{i}) {{ id title lengthSeconds status \
                         previewThumbnailURL(width: 440, height: 248) \
                         game {{ displayName }} \
                         owner {{ displayName profileImageURL(width: 70) roles {{ isPartner }} }} }}"
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            let query = format!("query StreamNookVodsByIds({params}) {{ {selections} }}");
            let variables: serde_json::Map<String, serde_json::Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| (format!("i{i}"), serde_json::Value::String(id.clone())))
                .collect();

            let resp = Self::gql_public_read(serde_json::json!({
                "operationName": "StreamNookVodsByIds",
                "query": query,
                "variables": variables,
            }))
            .await?;

            for (i, id) in chunk.iter().enumerate() {
                // Map back by ALIAS INDEX, never by the returned id: a deleted
                // video comes back as a bare `null` with no id to match on.
                let node = resp.pointer(&format!("/data/v{i}"));
                match node.filter(|v| !v.is_null()) {
                    None => gone.push(id.clone()),
                    Some(v) => {
                        let status = v
                            .get("status")
                            .and_then(|s| s.as_str())
                            .unwrap_or_default()
                            .to_string();
                        found.push(crate::services::vod_progress_service::VodRefresh {
                            video_id: id.clone(),
                            title: v
                                .get("title")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            channel_name: v
                                .pointer("/owner/displayName")
                                .and_then(|n| n.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            thumbnail_url: v
                                .get("previewThumbnailURL")
                                .and_then(|t| t.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            length_secs: v
                                .get("lengthSeconds")
                                .and_then(|n| n.as_f64())
                                .unwrap_or(0.0),
                            finished: status.eq_ignore_ascii_case("RECORDED"),
                            profile_image_url: v
                                .pointer("/owner/profileImageURL")
                                .and_then(|n| n.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            partner: v.pointer("/owner/roles/isPartner").and_then(|b| b.as_bool()),
                            game_name: v
                                .pointer("/game/displayName")
                                .and_then(|n| n.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                }
            }
        }

        Ok((found, gone))
    }

    /// Format a second count as a Helix-style duration string ("3h21m4s").
    /// Inverse of `fmt_duration_secs` for Helix's "1h2m3s" strings. None for
    /// anything that is not that shape.
    pub fn parse_duration_str(s: &str) -> Option<u32> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let mut total: u64 = 0;
        let mut num: u64 = 0;
        let mut saw_digit = false;
        for c in s.chars() {
            match c {
                '0'..='9' => {
                    num = num.checked_mul(10)?.checked_add(c as u64 - '0' as u64)?;
                    saw_digit = true;
                }
                'h' => {
                    total = total.checked_add(num.checked_mul(3600)?)?;
                    num = 0;
                }
                'm' => {
                    total = total.checked_add(num.checked_mul(60)?)?;
                    num = 0;
                }
                's' => {
                    total = total.checked_add(num)?;
                    num = 0;
                }
                _ => return None,
            }
        }
        if !saw_digit || num != 0 {
            return None;
        }
        Some(total.min(u32::MAX as u64) as u32)
    }

    fn fmt_duration_secs(total: u64) -> String {
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        if h > 0 {
            format!("{}h{}m{}s", h, m, s)
        } else if m > 0 {
            format!("{}m{}s", m, s)
        } else {
            format!("{}s", s)
        }
    }

    /// Update Chat Settings (Emote mode, Follower mode, Slow mode, etc.)
    pub async fn update_chat_settings(
        broadcaster_id: &str,
        settings: serde_json::Value,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/settings?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let response = client
            .patch(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&settings)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update chat settings: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to update chat settings"));
        }

        Ok(())
    }

    /// Clear all messages from chat
    pub async fn clear_chat(broadcaster_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/chat?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to clear chat: {}", error_text);
            return Err(anyhow::anyhow!("Failed to clear chat"));
        }

        Ok(())
    }

    /// Delete a specific chat message
    pub async fn delete_chat_message(broadcaster_id: &str, message_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/chat?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to delete message (HTTP {}): {}",
                status, error_text
            );
            // Surface Twitch's status + reason so the caller can tell apart a
            // restricted delete (broadcaster/other-mod message, >6h old) from a
            // scope/auth failure instead of an opaque "Failed to delete message".
            return Err(anyhow::anyhow!(
                "Twitch rejected delete (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Pin a chat message to the top of the broadcaster's chat room (mod action).
    ///
    /// Helix `PUT /helix/chat/pins` — every parameter rides in the query string
    /// and the body is empty. Uses the `moderator:manage:chat_messages` scope,
    /// the SAME scope `delete_chat_message` already holds, so no re-auth is
    /// needed. Only one mod-pinned message is active at a time; pinning a new one
    /// replaces the prior pin. `duration_seconds` is optional (Twitch applies its
    /// own default when omitted).
    pub async fn pin_chat_message(
        broadcaster_id: &str,
        message_id: &str,
        duration_seconds: Option<u32>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let mut url = format!(
            "https://api.twitch.tv/helix/chat/pins?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );
        if let Some(d) = duration_seconds {
            url.push_str(&format!("&duration_seconds={}", d));
        }

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to pin message (HTTP {}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!(
                "Twitch rejected pin (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Unpin a mod-pinned chat message (mod action).
    ///
    /// Helix `DELETE /helix/chat/pins?broadcaster_id=..&moderator_id=..&message_id=..`
    /// — `message_id` IS required (Twitch returns 400 "Missing required parameter
    /// message_id" without it). Same `moderator:manage:chat_messages` scope as
    /// pin/delete.
    pub async fn unpin_chat_message(broadcaster_id: &str, message_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/pins?broadcaster_id={}&moderator_id={}&message_id={}",
            broadcaster_id, moderator_id, message_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to unpin message (HTTP {}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!(
                "Twitch rejected unpin (HTTP {}): {}",
                status,
                error_text
            ));
        }

        Ok(())
    }

    /// Ban or timeout a user
    pub async fn ban_user(
        broadcaster_id: &str,
        target_user_id: &str,
        duration: Option<u32>,
        reason: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let mut payload = serde_json::json!({
            "data": {
                "user_id": target_user_id,
            }
        });

        if let Some(d) = duration {
            payload["data"]["duration"] = serde_json::json!(d);
        }

        if let Some(r) = reason {
            if !r.is_empty() {
                payload["data"]["reason"] = serde_json::json!(r);
            }
        }

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to ban/timeout user: {}", error_text);
            return Err(anyhow::anyhow!("Failed to ban or timeout user"));
        }

        Ok(())
    }

    /// Unban a user
    pub async fn unban_user(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/bans?broadcaster_id={}&moderator_id={}&user_id={}",
            broadcaster_id, moderator_id, target_user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to unban user: {}", error_text);
            return Err(anyhow::anyhow!("Failed to unban user"));
        }

        Ok(())
    }

    /// Add a channel moderator
    pub async fn add_channel_moderator(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to add moderator: {}", error_text);
            return Err(anyhow::anyhow!("Failed to add moderator"));
        }

        Ok(())
    }

    /// Remove a channel moderator
    pub async fn remove_channel_moderator(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove moderator: {}", error_text);
            return Err(anyhow::anyhow!("Failed to remove moderator"));
        }

        Ok(())
    }

    /// Add a channel VIP
    pub async fn add_channel_vip(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to add VIP: {}", error_text);
            return Err(anyhow::anyhow!("Failed to add VIP"));
        }

        Ok(())
    }

    /// Remove a channel VIP
    pub async fn remove_channel_vip(broadcaster_id: &str, user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove VIP: {}", error_text);
            return Err(anyhow::anyhow!("Failed to remove VIP"));
        }

        Ok(())
    }

    /// Update Suspicious User Status (Restrict/Monitor)
    /// Set the stream title and/or category (`channel:manage:broadcast`).
    /// A category is given by name and resolved through Helix `games?name=`;
    /// an unknown name is an error, never a silent no-op.
    pub async fn update_channel_info(
        broadcaster_id: &str,
        title: Option<&str>,
        game_name: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let mut payload = serde_json::Map::new();
        if let Some(t) = title {
            payload.insert("title".into(), serde_json::json!(t));
        }
        if let Some(name) = game_name {
            let resp = client
                .get("https://api.twitch.tv/helix/games")
                .query(&[("name", name)])
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            let json: serde_json::Value = resp.json().await?;
            let id = json
                .pointer("/data/0/id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or_else(|| anyhow::anyhow!("No Twitch category named \"{}\"", name))?;
            payload.insert("game_id".into(), serde_json::json!(id));
        }
        if payload.is_empty() {
            return Ok(());
        }
        let response = client
            .patch(format!(
                "https://api.twitch.tv/helix/channels?broadcaster_id={}",
                broadcaster_id
            ))
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&serde_json::Value::Object(payload))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            error!("[TwitchService] update_channel_info failed ({}): {}", status, text);
            return Err(anyhow::anyhow!(match status {
                401 | 403 => "Only the broadcaster can change the title or category".to_string(),
                _ => format!("Channel update failed ({})", status),
            }));
        }
        Ok(())
    }

    /// Allow or deny an AutoMod-held message (`moderator:manage:automod`).
    pub async fn resolve_automod_message(msg_id: &str, allow: bool) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let payload = serde_json::json!({
            "user_id": user_info.id,
            "msg_id": msg_id,
            "action": if allow { "ALLOW" } else { "DENY" },
        });
        let response = client
            .post("https://api.twitch.tv/helix/moderation/automod/message")
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] AutoMod resolve failed ({}): {}", status, error_text);
            return Err(anyhow::anyhow!(match status {
                400 => "That message is no longer held".to_string(),
                403 => "Not allowed to review AutoMod for this channel".to_string(),
                _ => format!("AutoMod request failed ({})", status),
            }));
        }
        Ok(())
    }

    pub async fn update_suspicious_user_status(
        broadcaster_id: &str,
        target_user_id: &str,
        status: &str,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/suspicious_users?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "user_id": target_user_id,
            "status": status // "RESTRICTED", "NO_TREATMENT", "ACTIVE_MONITORING"
        });

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update suspicious user status: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to update suspicious user status"));
        }

        Ok(())
    }

    /// Update User Chat Color
    pub async fn update_user_chat_color(user_id: &str, color: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/chat/color?user_id={}&color={}",
            user_id, color
        );

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update user chat color: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to update user chat color"));
        }

        Ok(())
    }

    /// Live status for an arbitrary set of channels, by numeric user id.
    ///
    /// This is what makes a FAVOURITE work when you don't follow the channel:
    /// `get_followed_streams` only ever answers for your follow list, so a
    /// favourite outside it is invisible without this.
    ///
    /// Helix facts this depends on (dev.twitch.tv, verified 2026-08-27):
    ///   - up to 100 `user_id` values per request, hence the chunking;
    ///   - **`first` defaults to 20**, so a 100-id request WITHOUT `first=100`
    ///     answers for only 20 of them and the rest read as offline;
    ///   - offline channels are simply absent from the response, so every row
    ///     that comes back is live;
    ///   - an app OR user token is required. There is no client-credentials
    ///     path here (see `get_token`), so signed out this returns an error
    ///     rather than pretending nobody is live.
    ///
    /// A failing chunk warns and is skipped rather than failing the whole call:
    /// partial liveness beats none, and the next sweep retries anyway.
    pub async fn get_streams_by_user_ids(user_ids: &[String]) -> Result<Vec<TwitchStream>> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }

        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let mut out: Vec<TwitchStream> = Vec::new();

        for chunk in user_ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("user_id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            // `first=100` is load-bearing, not decoration. See the note above.
            let url = format!("https://api.twitch.tv/helix/streams?{}&first=100", query);

            let response = match client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("[TwitchService] get_streams_by_user_ids request failed: {}", e);
                    continue;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                warn!(
                    "[TwitchService] get_streams_by_user_ids non-success {}: {}",
                    status, body
                );
                continue;
            }

            let json = match response.json::<serde_json::Value>().await {
                Ok(j) => j,
                Err(e) => {
                    warn!("[TwitchService] get_streams_by_user_ids bad JSON: {}", e);
                    continue;
                }
            };

            let Some(arr) = json.get("data").and_then(|d| d.as_array()) else {
                continue;
            };

            match serde_json::from_value::<Vec<TwitchStream>>(serde_json::Value::Array(
                arr.clone(),
            )) {
                Ok(mut streams) => {
                    // Helix omits offline channels entirely, so everything here
                    // is live. Stamp it: the frontend lists filter on `is_live`,
                    // and Helix doesn't ship the field.
                    for s in &mut streams {
                        s.is_live = Some(true);
                    }
                    out.append(&mut streams);
                }
                Err(e) => warn!(
                    "[TwitchService] get_streams_by_user_ids failed to parse rows: {}",
                    e
                ),
            }
        }

        Ok(out)
    }

    /// `user_id -> (login, display name, avatar)` for a batch of Twitch users.
    ///
    /// Used to back-fill identity for favourites saved before the identity
    /// sidecar existed: those are bare ids, and without a name and a face they
    /// can't be drawn in the offline roster at all.
    pub async fn users_by_ids(
        ids: &[String],
    ) -> std::collections::HashMap<String, (String, String, Option<String>)> {
        let mut out = std::collections::HashMap::new();
        if ids.is_empty() {
            return out;
        }

        let Ok(token) = Self::get_token().await else {
            return out;
        };
        let client = crate::services::http::client().clone();

        for chunk in ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/users?{}", query);

            let Ok(response) = client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }
            let Ok(json) = response.json::<serde_json::Value>().await else {
                continue;
            };
            let Some(arr) = json.get("data").and_then(|d| d.as_array()) else {
                continue;
            };
            for user in arr {
                let Some(id) = user.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let login = user
                    .get("login")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let display = user
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&login)
                    .to_string();
                let avatar = user
                    .get("profile_image_url")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                out.insert(id.to_string(), (login, display, avatar));
            }
        }

        out
    }

    /// Get User Chat Color for a batch of users, returning `user_id -> hex color`.
    /// Helix allows up to 100 ids per request and ignores duplicate/unknown ids.
    /// Degrades to an empty map on a missing token (anonymous viewing) or any
    /// request failure, so the chat render path always has a value to fall back
    /// on rather than surfacing an error.
    pub async fn get_user_chat_colors(
        user_ids: Vec<String>,
    ) -> std::collections::HashMap<String, String> {
        let mut out: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if user_ids.is_empty() {
            return out;
        }

        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => return out, // anonymous / logged-out: no color lookup available
        };
        let client = crate::services::http::client().clone();

        for chunk in user_ids.chunks(100) {
            let query = chunk
                .iter()
                .map(|id| format!("user_id={}", id))
                .collect::<Vec<_>>()
                .join("&");
            let url = format!("https://api.twitch.tv/helix/chat/color?{}", query);

            let response = match client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("[TwitchService] get_user_chat_colors request failed: {}", e);
                    continue;
                }
            };

            if !response.status().is_success() {
                let error_text = response.text().await.unwrap_or_default();
                warn!(
                    "[TwitchService] get_user_chat_colors non-success: {}",
                    error_text
                );
                continue;
            }

            let body: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    warn!("[TwitchService] get_user_chat_colors parse failed: {}", e);
                    continue;
                }
            };

            if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
                for item in data {
                    let uid = item.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
                    let color = item.get("color").and_then(|v| v.as_str()).unwrap_or("");
                    if !uid.is_empty() && !color.is_empty() {
                        out.insert(uid.to_string(), color.to_string());
                    }
                }
            }
        }

        out
    }

    /// Block user
    pub async fn block_user(target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;

        let url = format!(
            "https://api.twitch.tv/helix/users/blocks?target_user_id={}",
            target_user_id
        );

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to block user: {}", error_text);
            return Err(anyhow::anyhow!("Failed to block user"));
        }

        Ok(())
    }

    /// Unblock user
    pub async fn unblock_user(target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;

        let url = format!(
            "https://api.twitch.tv/helix/users/blocks?target_user_id={}",
            target_user_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to unblock user: {}", error_text);
            return Err(anyhow::anyhow!("Failed to unblock user"));
        }

        Ok(())
    }

    /// Get Channel Moderators
    pub async fn get_channel_moderators(broadcaster_id: &str) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/moderation/moderators?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to get moderators ({}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("Failed to get moderators: HTTP {}", status));
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            Ok(data.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Get Channel VIPs
    pub async fn get_channel_vips(broadcaster_id: &str) -> Result<Vec<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/channels/vips?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to get VIPs ({}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("Failed to get VIPs: HTTP {}", status));
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            Ok(data.clone())
        } else {
            Ok(Vec::new())
        }
    }

    /// Get channel chatters (moderators and VIPs) via GQL
    /// The Helix API requires broadcaster_id to match the access token user_id,
    /// so we use the GQL Mods/VIPs queries which work for any authenticated user.
    pub async fn get_chatters_by_role(channel_login: &str) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();

        // Build batch GQL request — both Mods and VIPs in one round trip
        let payload = serde_json::json!([
            {
                "operationName": "Mods",
                "variables": {
                    "login": channel_login
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "cb912a7e0789e0f8a4c85c25041a08324475831024d03d624172b59498caf085"
                    }
                }
            },
            {
                "operationName": "VIPs",
                "variables": {
                    "login": channel_login
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "612a574d07afe5db2f9e878e290225224a0b955e65b5d1235dcd4b68ff668218"
                    }
                }
            }
        ]);

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", TWITCH_GQL_CLIENT_ID)
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] GQL Mods/VIPs failed ({}): {}",
                status, error_text
            );
            return Err(anyhow::anyhow!("GQL Mods/VIPs failed: HTTP {}", status));
        }

        let json: serde_json::Value = response.json().await?;

        // Response is an array of two results: [ModsResponse, VIPsResponse]
        // Mods: data.user.mods.edges[].node.login
        // VIPs: data.user.vips.edges[].node.login
        let mut moderators: Vec<serde_json::Value> = Vec::new();
        let mut vips: Vec<serde_json::Value> = Vec::new();

        if let Some(arr) = json.as_array() {
            // Parse Mods response (index 0)
            if let Some(mods_data) = arr.first() {
                if let Some(edges) = mods_data
                    .get("data")
                    .and_then(|d| d.get("user"))
                    .and_then(|u| u.get("mods"))
                    .and_then(|m| m.get("edges"))
                    .and_then(|e| e.as_array())
                {
                    for edge in edges {
                        if let Some(node) = edge.get("node") {
                            moderators.push(node.clone());
                        }
                    }
                }
            }

            // Parse VIPs response (index 1)
            if let Some(vips_data) = arr.get(1) {
                if let Some(edges) = vips_data
                    .get("data")
                    .and_then(|d| d.get("user"))
                    .and_then(|u| u.get("vips"))
                    .and_then(|v| v.get("edges"))
                    .and_then(|e| e.as_array())
                {
                    for edge in edges {
                        if let Some(node) = edge.get("node") {
                            vips.push(node.clone());
                        }
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "moderators": moderators,
            "vips": vips
        }))
    }

    /// Get the full official chatters roster for a channel, grouped by role.
    ///
    /// Uses Helix Get Chatters, which requires the caller to be a moderator or the
    /// broadcaster of the channel and to hold the `moderator:read:chatters` scope.
    /// That endpoint returns logins only, so we compose it with the GQL Mods/VIPs
    /// lookup (`get_chatters_by_role`, which works for any authenticated user) to
    /// bucket each chatter into broadcaster / moderators / vips / viewers.
    pub async fn get_channel_chatters(
        broadcaster_id: &str,
        channel_login: &str,
    ) -> Result<serde_json::Value> {
        use std::collections::HashSet;

        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        // The roster endpoint requires moderator_id = the authenticated user's id.
        let moderator_id = Self::get_user_info().await?.id;

        // 1. Page through the official chatters roster (first=1000 is the max page).
        const MAX_PAGES: usize = 10; // ~10k chatters; surfaced via `truncated` if hit.
        let mut roster: Vec<serde_json::Value> = Vec::new();
        let mut cursor: Option<String> = None;
        let mut truncated = false;
        for page in 0..MAX_PAGES {
            let mut url = format!(
                "https://api.twitch.tv/helix/chat/chatters?broadcaster_id={}&moderator_id={}&first=1000",
                broadcaster_id, moderator_id
            );
            if let Some(ref c) = cursor {
                url.push_str(&format!("&after={}", c));
            }

            let response = client
                .get(&url)
                .header("Client-Id", CLIENT_ID)
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                error!(
                    "[TwitchService] Failed to get chatters ({}): {}",
                    status, error_text
                );
                // 401 here means the token predates the moderator:read:chatters scope;
                // surface a distinct signal so the UI can prompt a one-time re-login.
                if status.as_u16() == 401 {
                    return Err(anyhow::anyhow!("REAUTH"));
                }
                return Err(anyhow::anyhow!("Failed to get chatters: HTTP {}", status));
            }

            let json: serde_json::Value = response.json().await?;
            if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                roster.extend(data.iter().cloned());
            }
            cursor = json
                .get("pagination")
                .and_then(|p| p.get("cursor"))
                .and_then(|c| c.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            if cursor.is_none() {
                break;
            }
            if page == MAX_PAGES - 1 {
                truncated = true;
            }
        }

        // 2. Role sets from the GQL Mods/VIPs lookup (best-effort; an empty result
        //    just means everyone falls into the viewers bucket).
        let mut mod_logins: HashSet<String> = HashSet::new();
        let mut vip_logins: HashSet<String> = HashSet::new();
        if let Ok(roles) = Self::get_chatters_by_role(channel_login).await {
            if let Some(mods) = roles.get("moderators").and_then(|m| m.as_array()) {
                for m in mods {
                    if let Some(login) = m.get("login").and_then(|l| l.as_str()) {
                        mod_logins.insert(login.to_lowercase());
                    }
                }
            }
            if let Some(vips) = roles.get("vips").and_then(|v| v.as_array()) {
                for v in vips {
                    if let Some(login) = v.get("login").and_then(|l| l.as_str()) {
                        vip_logins.insert(login.to_lowercase());
                    }
                }
            }
        }

        // 3. Bucket each chatter by role.
        fn login_key(v: &serde_json::Value) -> String {
            v.get("user_login")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_lowercase()
        }

        let broadcaster_login = channel_login.to_lowercase();
        let mut broadcaster: Vec<serde_json::Value> = Vec::new();
        let mut moderators: Vec<serde_json::Value> = Vec::new();
        let mut vips: Vec<serde_json::Value> = Vec::new();
        let mut viewers: Vec<serde_json::Value> = Vec::new();

        for chatter in &roster {
            let login = login_key(chatter);
            if login == broadcaster_login {
                broadcaster.push(chatter.clone());
            } else if mod_logins.contains(&login) {
                moderators.push(chatter.clone());
            } else if vip_logins.contains(&login) {
                vips.push(chatter.clone());
            } else {
                viewers.push(chatter.clone());
            }
        }

        // Alphabetical within each bucket, matching the native viewer list.
        moderators.sort_by_cached_key(login_key);
        vips.sort_by_cached_key(login_key);
        viewers.sort_by_cached_key(login_key);

        let total = roster.len();
        Ok(serde_json::json!({
            "broadcaster": broadcaster,
            "moderators": moderators,
            "vips": vips,
            "viewers": viewers,
            "total": total,
            "truncated": truncated,
        }))
    }

    /// Send Chat Announcement
    pub async fn send_chat_announcement(
        broadcaster_id: &str,
        message: &str,
        color: Option<&str>,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/announcements?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let mut payload = serde_json::json!({
            "message": message
        });

        if let Some(c) = color {
            payload["color"] = serde_json::json!(c);
        }

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to send announcement: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to send chat announcement"));
        }

        Ok(())
    }

    /// Send Shoutout
    pub async fn send_shoutout(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/chat/shoutouts?from_broadcaster_id={}&to_broadcaster_id={}&moderator_id={}",
            broadcaster_id, target_user_id, moderator_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to send shoutout: {}", error_text);
            return Err(anyhow::anyhow!("Failed to send shoutout"));
        }

        Ok(())
    }

    /// Start Commercial
    pub async fn start_commercial(broadcaster_id: &str, length: u32) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = "https://api.twitch.tv/helix/channels/commercial";

        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "length": length
        });

        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to start commercial: {}", error_text);
            return Err(anyhow::anyhow!("Failed to start commercial"));
        }

        Ok(())
    }

    /// Create a poll. Broadcaster-only (`channel:manage:polls`).
    ///
    /// `duration` is seconds (Twitch allows 15..1800). Channel-point voting is
    /// only sent when a cost is set, because Twitch rejects the pair when the
    /// enable flag is true with a zero cost.
    pub async fn create_poll(
        broadcaster_id: &str,
        title: &str,
        choices: &[String],
        duration: u32,
        points_per_vote: Option<u32>,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let mut payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "title": title,
            "duration": duration,
            "choices": choices
                .iter()
                .map(|c| serde_json::json!({ "title": c }))
                .collect::<Vec<_>>(),
        });
        if let Some(cost) = points_per_vote.filter(|c| *c > 0) {
            payload["channel_points_voting_enabled"] = serde_json::json!(true);
            payload["channel_points_per_vote"] = serde_json::json!(cost);
        }

        Self::helix_json_post("https://api.twitch.tv/helix/polls", &token, payload, "create poll")
            .await
    }

    /// Add an AutoMod blocked term. Needs `moderator:manage:blocked_terms`.
    ///
    /// Twitch takes the ids as query params and only the phrase in the body.
    pub async fn add_blocked_term(broadcaster_id: &str, text: &str) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let moderator_id = Self::get_user_info().await?.id;
        let url = format!(
            "https://api.twitch.tv/helix/moderation/blocked_terms?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );
        Self::helix_json_post(&url, &token, serde_json::json!({ "text": text }), "add blocked term")
            .await
    }

    /// Remove a blocked term by its phrase.
    ///
    /// The delete endpoint only takes the term's id, so this looks the phrase up
    /// first. Matching is case-insensitive because Twitch stores terms lowercased
    /// but echoes back whatever case they were added in.
    pub async fn remove_blocked_term(broadcaster_id: &str, text: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let moderator_id = Self::get_user_info().await?.id;
        let client = crate::services::http::client().clone();

        let list_url = format!(
            "https://api.twitch.tv/helix/moderation/blocked_terms?broadcaster_id={}&moderator_id={}&first=100",
            broadcaster_id, moderator_id
        );
        let listed = client
            .get(&list_url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let value = Self::helix_json_result(listed, "list blocked terms").await?;

        let wanted = text.trim().to_lowercase();
        let id = value["data"]
            .as_array()
            .and_then(|rows| {
                rows.iter().find(|r| {
                    r["text"].as_str().map(|t| t.trim().to_lowercase()) == Some(wanted.clone())
                })
            })
            .and_then(|r| r["id"].as_str().map(String::from))
            .ok_or_else(|| anyhow::anyhow!("\"{}\" is not in the blocked terms list", text.trim()))?;

        let del_url = format!(
            "https://api.twitch.tv/helix/moderation/blocked_terms?broadcaster_id={}&moderator_id={}&id={}",
            broadcaster_id, moderator_id, id
        );
        let response = client
            .delete(&del_url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to remove blocked term: {}", body);
            return Err(anyhow::anyhow!("Could not remove that blocked term"));
        }
        Ok(())
    }

    /// The channel's currently-running poll, if any.
    ///
    /// Twitch exposes no GQL read for polls (the web client learns them over
    /// PubSub), which is why the overlay is event-fed. Helix does have one, and
    /// it became reachable when `channel:manage:polls` was added — so commands
    /// that need the live poll id no longer have to guess at it.
    pub async fn get_active_poll(broadcaster_id: &str) -> Result<Option<serde_json::Value>> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let url = format!(
            "https://api.twitch.tv/helix/polls?broadcaster_id={}&first=1",
            broadcaster_id
        );
        let response = client
            .get(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;
        let value = Self::helix_json_result(response, "get polls").await?;
        // Helix returns the most recent poll regardless of state, so an ended
        // one must not be reported as live.
        let poll = value["data"].get(0).cloned().filter(|p| {
            matches!(p["status"].as_str(), Some("ACTIVE"))
        });
        Ok(poll)
    }

    /// End a poll. `status` is TERMINATED (show the result) or ARCHIVED (hide it).
    pub async fn end_poll(broadcaster_id: &str, poll_id: &str, status: &str) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "id": poll_id,
            "status": status,
        });
        Self::helix_json_patch("https://api.twitch.tv/helix/polls", &token, payload, "end poll").await
    }

    /// Create a prediction. Broadcaster-only (`channel:manage:predictions`).
    ///
    /// Note the field is `prediction_window`, not `prediction_window_seconds`.
    pub async fn create_prediction(
        broadcaster_id: &str,
        title: &str,
        outcomes: &[String],
        window: u32,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "title": title,
            "prediction_window": window,
            "outcomes": outcomes
                .iter()
                .map(|o| serde_json::json!({ "title": o }))
                .collect::<Vec<_>>(),
        });
        Self::helix_json_post(
            "https://api.twitch.tv/helix/predictions",
            &token,
            payload,
            "create prediction",
        )
        .await
    }

    /// Lock, resolve or cancel a prediction. `status` is LOCKED, RESOLVED or
    /// CANCELED; RESOLVED additionally requires the winning outcome id.
    pub async fn end_prediction(
        broadcaster_id: &str,
        prediction_id: &str,
        status: &str,
        winning_outcome_id: Option<&str>,
    ) -> Result<serde_json::Value> {
        let token = Self::get_token().await?;
        let mut payload = serde_json::json!({
            "broadcaster_id": broadcaster_id,
            "id": prediction_id,
            "status": status,
        });
        if let Some(outcome) = winning_outcome_id {
            payload["winning_outcome_id"] = serde_json::json!(outcome);
        }
        Self::helix_json_patch(
            "https://api.twitch.tv/helix/predictions",
            &token,
            payload,
            "end prediction",
        )
        .await
    }

    /// POST a JSON body to Helix and return the parsed response. Surfaces
    /// Twitch's own error text, which names the offending field on a 400.
    async fn helix_json_post(
        url: &str,
        token: &str,
        payload: serde_json::Value,
        what: &str,
    ) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        Self::helix_json_result(response, what).await
    }

    async fn helix_json_patch(
        url: &str,
        token: &str,
        payload: serde_json::Value,
        what: &str,
    ) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();
        let response = client
            .patch(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .json(&payload)
            .send()
            .await?;
        Self::helix_json_result(response, what).await
    }

    async fn helix_json_result(
        response: reqwest::Response,
        what: &str,
    ) -> Result<serde_json::Value> {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            error!("[TwitchService] Failed to {}: {} {}", what, status, body);
            // Twitch puts a human-readable reason in `message`; pass it through
            // so the composer can show why rather than a generic failure.
            let reason = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Err(anyhow::anyhow!("{}", reason));
        }
        Ok(serde_json::from_str(&body).unwrap_or(serde_json::Value::Null))
    }

    /// Start Raid
    pub async fn start_raid(broadcaster_id: &str, target_user_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/raids?from_broadcaster_id={}&to_broadcaster_id={}",
            broadcaster_id, target_user_id
        );

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to start raid: {}", error_text);
            return Err(anyhow::anyhow!("Failed to start raid"));
        }

        Ok(())
    }

    /// Cancel Raid
    pub async fn cancel_raid(broadcaster_id: &str) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = format!(
            "https://api.twitch.tv/helix/raids?broadcaster_id={}",
            broadcaster_id
        );

        let response = client
            .delete(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .send()
            .await?;

        if !response.status().is_success() && response.status() != 204 {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to cancel raid: {}", error_text);
            return Err(anyhow::anyhow!("Failed to cancel raid"));
        }

        Ok(())
    }

    /// Create Stream Marker
    pub async fn create_stream_marker(user_id: &str, description: Option<&str>) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();

        let url = "https://api.twitch.tv/helix/streams/markers";

        let mut payload = serde_json::json!({
            "user_id": user_id
        });

        if let Some(desc) = description {
            if !desc.is_empty() {
                payload["description"] = serde_json::json!(desc);
            }
        }

        let response = client
            .post(url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to create stream marker: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to create stream marker"));
        }

        Ok(())
    }

    /// Warn a chat user (Helix: POST /moderation/warnings)
    pub async fn warn_chat_user(
        broadcaster_id: &str,
        target_user_id: &str,
        reason: &str,
    ) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/warnings?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "data": {
                "user_id": target_user_id,
                "reason": reason
            }
        });

        let response = client
            .post(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("[TwitchService] Failed to warn user: {}", error_text);
            return Err(anyhow::anyhow!("Failed to warn user"));
        }

        Ok(())
    }

    /// Update Shield Mode (Helix: PUT /moderation/shield_mode)
    pub async fn update_shield_mode(broadcaster_id: &str, is_active: bool) -> Result<()> {
        let token = Self::get_token().await?;
        let client = crate::services::http::client().clone();
        let user_info = Self::get_user_info().await?;
        let moderator_id = &user_info.id;

        let url = format!(
            "https://api.twitch.tv/helix/moderation/shield_mode?broadcaster_id={}&moderator_id={}",
            broadcaster_id, moderator_id
        );

        let payload = serde_json::json!({
            "is_active": is_active
        });

        let response = client
            .put(&url)
            .header("Client-Id", CLIENT_ID)
            .header(AUTHORIZATION, format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!(
                "[TwitchService] Failed to update shield mode: {}",
                error_text
            );
            return Err(anyhow::anyhow!("Failed to update shield mode"));
        }

        Ok(())
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    #[test]
    fn gql_language_normalizes_to_helix_form() {
        assert_eq!(normalize_gql_language("FR"), "fr");
        assert_eq!(normalize_gql_language("EN"), "en");
        assert_eq!(normalize_gql_language("ZH_HK"), "zh-hk");
        assert_eq!(normalize_gql_language("OTHER"), "other");
    }

    #[test]
    fn recommended_edges_map_to_streams() {
        let edges: Vec<serde_json::Value> = serde_json::from_str(
            r#"[
            {"node": {
                "id": "317579120484", "title": "ranked", "viewersCount": 5701,
                "language": "FR", "createdAt": "2026-08-25T18:20:15Z",
                "previewImageURL": "https://static-cdn.jtvnw.net/previews-ttv/live_user_kamet0-{width}x{height}.jpg",
                "freeformTags": [{"name": "Francais"}],
                "game": {"id": "21779", "name": "League of Legends", "displayName": "League of Legends"},
                "broadcaster": {"id": "27115917", "login": "kamet0", "displayName": "Kamet0",
                    "profileImageURL": "https://example/p.png",
                    "roles": {"isPartner": true, "isAffiliate": false}}
            }},
            {"node": {"id": "1", "title": "no broadcaster", "viewersCount": 1, "broadcaster": null}},
            {"node": {
                "id": "2", "title": "cantonese", "viewersCount": 9, "language": "ZH_HK",
                "previewImageURL": "", "game": null,
                "broadcaster": {"id": "5", "login": "hk_chan", "displayName": "",
                    "roles": {"isPartner": false, "isAffiliate": true}}
            }}
        ]"#,
        )
        .unwrap();

        let streams = TwitchService::map_recommended_edges(&edges);
        assert_eq!(streams.len(), 2, "null-broadcaster edge must be skipped");

        let first = &streams[0];
        assert_eq!(first.user_login, "kamet0");
        assert_eq!(first.user_name, "Kamet0");
        assert_eq!(first.viewer_count, 5701);
        assert_eq!(first.game_id, "21779");
        assert_eq!(first.game_name, "League of Legends");
        assert_eq!(first.language.as_deref(), Some("fr"));
        assert_eq!(first.broadcaster_type.as_deref(), Some("partner"));
        assert_eq!(first.started_at, "2026-08-25T18:20:15Z");
        assert_eq!(first.is_live, Some(true));
        assert_eq!(first.tags.as_deref(), Some(&["Francais".to_string()][..]));

        let second = &streams[1];
        assert_eq!(second.user_name, "hk_chan", "empty displayName falls back to login");
        assert_eq!(second.language.as_deref(), Some("zh-hk"));
        assert_eq!(second.broadcaster_type.as_deref(), Some("affiliate"));
        assert_eq!(second.game_name, "");
    }
}
