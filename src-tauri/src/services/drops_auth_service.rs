// HTTP_CLIENT below kept as a thin file-local alias for the shared client so
// existing `HTTP_CLIENT.clone()` call sites continue to work; the underlying
// instance is the global default in `crate::services::http`.
lazy_static::lazy_static! { static ref HTTP_CLIENT: reqwest::Client = crate::services::http::client().clone(); }

use crate::services::token_vault;
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

// Twitch Android App credentials
const DROPS_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
const DROPS_TOKEN_FILE_NAME: &str = ".twitch_drops_token";

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct StorableDropsToken {
    access_token: String,
    // NOTE: refresh_token is not used - Android client doesn't support refresh without client secret
    // We store it anyway for potential future use, but never attempt to refresh
    refresh_token: String,
    // NOTE: expires_at is not used - we simply use the token until it's rejected by Twitch
    // This matches the official Android app's behavior
    expires_at: i64, // Unix timestamp (unused but kept for compatibility)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropsDeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

pub struct DropsAuthService;

impl DropsAuthService {
    fn get_token_file_path() -> Result<PathBuf> {
        // Mobile: the app-private sandbox dir (see services::app_paths). This
        // resolver was missed when the other file-backed stores were routed
        // through it, so on Android `dirs::config_dir()` returned None, the
        // drops token was never written, and every drops call reported "not
        // authenticated" immediately after a successful device-code connect.
        let mut path = match crate::services::app_paths::mobile_base() {
            Some(base) => base,
            None => dirs::config_dir()
                .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?,
        };
        path.push("StreamNook");

        if !path.exists() {
            fs::create_dir_all(&path)?;
        }

        path.push(DROPS_TOKEN_FILE_NAME);
        Ok(path)
    }

    fn store_token_to_file(token: &StorableDropsToken) -> Result<()> {
        let path = Self::get_token_file_path()?;
        token_vault::store_json(&path, token)?;
        debug!("[DROPS_AUTH] Token sealed to {:?}", path);
        Ok(())
    }

    fn delete_token_file() -> Result<()> {
        token_vault::remove(&Self::get_token_file_path()?)
    }

    fn load_token() -> Result<StorableDropsToken> {
        token_vault::load_json(&Self::get_token_file_path()?)?
            .ok_or_else(|| anyhow::anyhow!("No stored drops token"))
    }

    /// The page the browser lands on once the grant completes. Registered for this
    /// client, so it must match byte for byte.
    const REDIRECT_URI_ENCODED: &'static str = "https%3A%2F%2Fwww.twitch.tv%2F";

    /// Where the sign-in overlay is pointed. The token comes back on the fragment
    /// of the redirect, which the overlay's URL reporter already surfaces.
    pub fn authorize_url() -> String {
        format!(
            "https://id.twitch.tv/oauth2/authorize?client_id={}&response_type=token&redirect_uri={}&scope=",
            DROPS_CLIENT_ID,
            Self::REDIRECT_URI_ENCODED
        )
    }

    /// Pull the access token out of a landed redirect. Returns None for any URL
    /// that is not one, so callers can hand it every navigation they observe.
    pub fn access_token_from_redirect(url: &str) -> Option<String> {
        let fragment = url.split_once('#')?.1;
        fragment.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            if key == "access_token" && !value.is_empty() {
                Some(value.to_string())
            } else {
                None
            }
        })
    }

    /// Persist a token obtained without a refresh companion. Mirrors the device
    /// flow's storage so every reader downstream is unchanged.
    pub async fn store_access_token(access_token: String) -> Result<()> {
        let token = StorableDropsToken {
            access_token,
            refresh_token: String::new(),
            expires_at: 0,
        };

        Self::store_token_to_file(&token)
            .map_err(|e| anyhow::anyhow!("could not store the drops token: {e:#}"))
    }

    /// Start the device code flow for drops authentication
    pub async fn start_device_flow() -> Result<DropsDeviceCodeInfo> {
        let client = HTTP_CLIENT.clone();

        let params = [
            ("client_id", DROPS_CLIENT_ID),
            ("scopes", ""), // NO SCOPES - this is critical!
        ];

        debug!("[DROPS_AUTH] Starting device flow with Android app client ID");
        debug!("[DROPS_AUTH] Client ID: {}", DROPS_CLIENT_ID);
        debug!("[DROPS_AUTH] Scopes: (empty)");

        let response = client
            .post("https://id.twitch.tv/oauth2/device")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to start drops device flow: {}",
                error_text
            ));
        }

        let device_response: DeviceCodeResponse = response.json().await?;

        debug!("[DROPS_AUTH] Device flow started successfully");
        debug!("[DROPS_AUTH] User code: {}", device_response.user_code);
        debug!(
            "[DROPS_AUTH] Verification URI: {}",
            device_response.verification_uri
        );

        let verification_uri = if device_response.verification_uri.contains("device-code=") {
            device_response.verification_uri
        } else {
            let sep = if device_response.verification_uri.contains('?') {
                '&'
            } else {
                '?'
            };
            format!(
                "{}{}device-code={}",
                device_response.verification_uri, sep, device_response.user_code
            )
        };

        Ok(DropsDeviceCodeInfo {
            user_code: device_response.user_code,
            verification_uri,
            device_code: device_response.device_code,
            interval: device_response.interval,
            expires_in: device_response.expires_in,
        })
    }

    /// Poll for the token after the user has entered the code
    pub async fn poll_for_token(
        device_code: &str,
        interval: u64,
        expires_in: u64,
    ) -> Result<String> {
        let client = HTTP_CLIENT.clone();
        let start_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let expiry_time = start_time + expires_in;
        let mut poll_interval = interval;

        debug!("[DROPS_AUTH] Starting token polling...");

        loop {
            let current_time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            if current_time >= expiry_time {
                return Err(anyhow::anyhow!(
                    "Device code expired. Please try logging in again."
                ));
            }

            tokio::time::sleep(Duration::from_secs(poll_interval)).await;

            let params = [
                ("client_id", DROPS_CLIENT_ID),
                ("scopes", ""), // NO SCOPES
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

                let expires_at = Utc::now()
                    + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

                let storable_token = StorableDropsToken {
                    access_token: token_response.access_token.clone(),
                    refresh_token: token_response.refresh_token.clone().unwrap_or_default(),
                    expires_at: expires_at.timestamp(),
                };

                if let Err(e) = Self::store_token_to_file(&storable_token) {
                    // Still continue since we have the token in memory
                    error!("[DROPS_AUTH] Failed to store token: {:?}", e);
                }

                debug!(
                    "[DROPS_AUTH] Access token (first 10 chars): {}...",
                    &token_response.access_token[..10.min(token_response.access_token.len())]
                );

                return Ok(token_response.access_token);
            }

            let error_text = response.text().await?;

            if error_text.contains("authorization_pending") {
                // User hasn't authorized yet, continue polling
                debug!("[DROPS_AUTH] Waiting for user authorization...");
                continue;
            } else if error_text.contains("slow_down") {
                // Twitch wants us to slow down
                poll_interval += 2;
                debug!(
                    "[DROPS_AUTH] Slowing down polling interval to {} seconds",
                    poll_interval
                );
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

    /// Logout - delete the drops token
    pub async fn logout() -> Result<()> {
        Self::delete_token_file()?;
        // Settled drop ids are per ACCOUNT. Carrying them into the next sign-in
        // would skip the auto-claim for drops that account has genuinely not
        // claimed, which fails silently because a skipped drop looks identical
        // to one already collected.
        crate::services::drops_service::clear_attempted_claims();
        debug!("[DROPS_AUTH] Drops logout complete - all tokens cleared");
        Ok(())
    }

    /// Refresh the drops token
    async fn refresh_token(refresh_token: &str) -> Result<StorableDropsToken> {
        let client = HTTP_CLIENT.clone();
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", DROPS_CLIENT_ID),
        ];

        debug!("[DROPS_AUTH] Refreshing drops token...");

        let response = client
            .post("https://id.twitch.tv/oauth2/token")
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            let error_msg = format!("Failed to refresh drops token: {}", error_text);

            // If refresh fails due to missing client secret or other OAuth issues,
            // delete the stored token so the user can re-authenticate
            if error_text.contains("client secret") || error_text.contains("invalid") {
                error!("[DROPS_AUTH] Token refresh failed - clearing stored tokens");
                error!("[DROPS_AUTH] Error: {}", error_text);
                let _ = Self::delete_token_file();
                return Err(anyhow::anyhow!(
                    "{}\n\nPlease log in again for drops functionality.",
                    error_msg
                ));
            }

            return Err(anyhow::anyhow!(error_msg));
        }

        let token_response: TokenResponse = response.json().await?;
        let expires_at =
            Utc::now() + ChronoDuration::seconds(token_response.expires_in.unwrap_or(3600) as i64);

        let new_storable_token = StorableDropsToken {
            access_token: token_response.access_token,
            refresh_token: token_response
                .refresh_token
                .unwrap_or_else(|| refresh_token.to_string()),
            expires_at: expires_at.timestamp(),
        };

        // Store the refreshed token
        Self::store_token_to_file(&new_storable_token)?;

        debug!("[DROPS_AUTH] Token refreshed successfully");

        Ok(new_storable_token)
    }

    /// Get the current drops token
    /// NOTE: We don't attempt token refresh because the Android client ID doesn't support it
    /// without a client secret. Instead, we use the token until Twitch rejects it (401),
    /// at which point validate_token() will delete it and require re-authentication.
    /// This matches the official Android app's behavior.
    pub async fn get_token() -> Result<String> {
        // NOTE: We don't check expires_at here - just use the token until it fails
        // Twitch will reject it with 401 when it's actually invalid
        match Self::load_token() {
            Ok(token) => Ok(token.access_token),
            Err(e) => {
                debug!("[DROPS_AUTH] No usable stored token: {:#}", e);
                Err(anyhow::anyhow!(
                    "Not authenticated for drops. Please log in to Twitch for drops functionality."
                ))
            }
        }
    }

    /// Check if the user is authenticated for drops.
    ///
    /// Uses get_token(), which reads the same sealed file the connect flow
    /// writes, so the answer is right immediately after a successful connect.
    pub async fn is_authenticated() -> bool {
        Self::get_token().await.is_ok()
    }

    /// Validate the current token
    pub async fn validate_token() -> Result<bool> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };

        let client = HTTP_CLIENT.clone();
        let response = client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", token))
            .send()
            .await?;

        if response.status() == 401 {
            // Token is invalid, delete it
            debug!("[DROPS_AUTH] Token validation failed (401) - clearing stored tokens");
            let _ = Self::delete_token_file();
            return Ok(false);
        }

        Ok(response.status().is_success())
    }
}
