use anyhow::Result;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

// 7TV API endpoint
const SEVENTV_GQL_URL: &str = "https://7tv.io/v4/gql";
const SEVENTV_TOKEN_FILE_NAME: &str = ".seventv_token";

lazy_static::lazy_static! {
    static ref SEVENTV_TOKEN: Arc<RwLock<Option<StorableSevenTVToken>>> = Arc::new(RwLock::new(None));
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct StorableSevenTVToken {
    pub access_token: String,
    pub user_id: String,   // 7TV user ID
    pub twitch_id: String, // Associated Twitch ID
    pub created_at: i64,   // Unix timestamp
}

#[derive(Debug, Clone, Serialize)]
pub struct SevenTVAuthStatus {
    pub is_authenticated: bool,
    pub user_id: Option<String>,
    pub twitch_id: Option<String>,
}

pub struct SevenTVAuthService;

impl SevenTVAuthService {
    fn get_token_file_path() -> Result<PathBuf> {
        // Mobile: app-private sandbox dir; desktop keeps dirs::config_dir.
        let mut path = match crate::services::app_paths::mobile_base() {
            Some(base) => base.join("StreamNook"),
            None => {
                let mut p = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
                p.push("StreamNook");
                p
            }
        };

        if !path.exists() {
            fs::create_dir_all(&path)?;
        }

        path.push(SEVENTV_TOKEN_FILE_NAME);
        Ok(path)
    }

    fn store_token_to_file(token: &StorableSevenTVToken) -> Result<()> {
        let path = Self::get_token_file_path()?;
        let token_json = serde_json::to_string(token)?;

        // Simple XOR encryption with a fixed key for basic obfuscation
        let key: Vec<u8> = "StreamNook7TVKey2024"
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
        debug!("[7TV_AUTH] Token saved to file: {:?}", path);
        Ok(())
    }

    fn load_token_from_file() -> Result<StorableSevenTVToken> {
        let path = Self::get_token_file_path()?;

        if !path.exists() {
            return Err(anyhow::anyhow!("7TV token file does not exist"));
        }

        let encrypted = fs::read(&path)?;

        // Decrypt using the same XOR method
        let key: Vec<u8> = "StreamNook7TVKey2024"
            .bytes()
            .cycle()
            .take(encrypted.len())
            .collect();
        let decrypted: String = encrypted
            .iter()
            .zip(key.iter())
            .map(|(a, b)| (a ^ b) as char)
            .collect();

        let token: StorableSevenTVToken = serde_json::from_str(&decrypted)?;
        Ok(token)
    }

    fn delete_token_file() -> Result<()> {
        let path = Self::get_token_file_path()?;
        if path.exists() {
            fs::remove_file(&path)?;
            debug!("[7TV_AUTH] 7TV token file deleted: {:?}", path);
        }
        Ok(())
    }

    /// Store a 7TV token (called after OAuth flow captures the token)
    pub async fn store_token(
        access_token: String,
        user_id: String,
        twitch_id: String,
    ) -> Result<()> {
        let storable_token = StorableSevenTVToken {
            access_token,
            user_id,
            twitch_id,
            created_at: chrono::Utc::now().timestamp(),
        };

        // Store to file
        Self::store_token_to_file(&storable_token)?;

        // Cache in memory
        let mut cached = SEVENTV_TOKEN.write().await;
        *cached = Some(storable_token);

        debug!("[7TV_AUTH] 7TV token stored successfully");
        Ok(())
    }

    /// Get the current 7TV token
    pub async fn get_token() -> Result<String> {
        // Check memory cache first
        {
            let cached = SEVENTV_TOKEN.read().await;
            if let Some(token) = cached.as_ref() {
                return Ok(token.access_token.clone());
            }
        }

        // Try to load from file
        match Self::load_token_from_file() {
            Ok(token) => {
                // Cache in memory for next time
                let mut cached = SEVENTV_TOKEN.write().await;
                *cached = Some(token.clone());
                Ok(token.access_token)
            }
            Err(_) => Err(anyhow::anyhow!(
                "Not authenticated with 7TV. Please connect your 7TV account."
            )),
        }
    }

    /// Get full token info (including user IDs)
    pub async fn get_token_info() -> Result<StorableSevenTVToken> {
        // Check memory cache first
        {
            let cached = SEVENTV_TOKEN.read().await;
            if let Some(token) = cached.as_ref() {
                return Ok(token.clone());
            }
        }

        // Try to load from file
        match Self::load_token_from_file() {
            Ok(token) => {
                // Cache in memory for next time
                let mut cached = SEVENTV_TOKEN.write().await;
                *cached = Some(token.clone());
                Ok(token)
            }
            Err(e) => Err(e),
        }
    }

    /// Check if authenticated
    pub async fn is_authenticated() -> bool {
        Self::get_token().await.is_ok()
    }

    /// Decode the JWT exp claim without signature verification (fast-path expiry check).
    /// Returns true if the token is expired or malformed.
    fn is_token_expired(access_token: &str) -> bool {
        // JWT format: header.payload.signature
        let parts: Vec<&str> = access_token.split('.').collect();
        if parts.len() != 3 {
            return true; // Malformed → treat as expired
        }

        let payload_bytes = match URL_SAFE_NO_PAD.decode(parts[1]) {
            Ok(b) => b,
            Err(_) => return true,
        };

        let payload: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
            Ok(v) => v,
            Err(_) => return true,
        };

        if let Some(exp) = payload.get("exp").and_then(|v| v.as_i64()) {
            let now = chrono::Utc::now().timestamp();
            exp < now
        } else {
            false // No exp claim → don't assume expired
        }
    }

    /// Get auth status with details.
    /// Uses local JWT exp decode for instant validation (no network).
    pub async fn get_auth_status() -> SevenTVAuthStatus {
        match Self::get_token_info().await {
            Ok(token) => {
                if Self::is_token_expired(&token.access_token) {
                    debug!("[7TV_AUTH] Stored token has expired (JWT exp) - auto-clearing");
                    let _ = Self::logout().await;
                    SevenTVAuthStatus {
                        is_authenticated: false,
                        user_id: None,
                        twitch_id: None,
                    }
                } else {
                    SevenTVAuthStatus {
                        is_authenticated: true,
                        user_id: Some(token.user_id),
                        twitch_id: Some(token.twitch_id),
                    }
                }
            }
            Err(_) => SevenTVAuthStatus {
                is_authenticated: false,
                user_id: None,
                twitch_id: None,
            },
        }
    }

    /// Validate the current token by making a test request
    pub async fn validate_token() -> Result<bool> {
        let token = match Self::get_token().await {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };

        let client = crate::services::http::client().clone();

        // Make a simple authenticated query to verify the token works
        let query = r#"{ users { me { id } } }"#;

        let response = client
            .post(SEVENTV_GQL_URL)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({
                "query": query
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            // Token might be invalid, clear it
            debug!("[7TV_AUTH] Token validation failed - clearing stored token");
            let _ = Self::logout().await;
            return Ok(false);
        }

        let response_json: serde_json::Value = response.json().await?;

        // Check if we got user data (token is valid)
        if response_json["data"]["users"]["me"]["id"].is_string() {
            Ok(true)
        } else {
            // Token invalid
            let _ = Self::logout().await;
            Ok(false)
        }
    }

    /// Logout - delete the 7TV token
    pub async fn logout() -> Result<()> {
        Self::delete_token_file()?;

        // Clear memory cache
        let mut cached = SEVENTV_TOKEN.write().await;
        *cached = None;

        debug!("[7TV_AUTH] 7TV logout complete - token cleared");
        Ok(())
    }

    /// Get the 7TV OAuth login URL
    /// User needs to visit this URL to authenticate, then we capture the token.
    ///
    /// 7TV's current (SvelteKit) website no longer auto-logs-in via the old
    /// `7tv.app/?login=true` page (that URL now just serves the homepage and
    /// never mints a token). Login is a real OAuth round-trip: this endpoint
    /// 303-redirects into Twitch OAuth, Twitch returns to `7tv.app/login/callback`,
    /// the callback finalizes the session and writes the JWT to
    /// `localStorage['7tv-token']` before redirecting to a normal 7tv.app page.
    /// The capture script (which polls that localStorage key on every page in the
    /// login window) then picks it up from the final authenticated page.
    pub fn get_login_url() -> String {
        "https://api.7tv.app/v4/auth/login?platform=twitch".to_string()
    }

    // ── Per-account 7TV tokens (for linked secondary accounts) ───────────────
    // Each linked account's 7TV token lives in its own obfuscated file keyed by
    // Twitch user id, separate from the primary's single `.seventv_token`.
    // Connected through an incognito login window so the alt's Twitch session
    // never touches the main's. The primary continues to use the methods above.

    fn xor_obfuscate(data: &[u8]) -> Vec<u8> {
        let key = b"StreamNook7TVKey2024";
        data.iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect()
    }

    fn account_token_file_path(twitch_id: &str) -> Result<PathBuf> {
        // Mobile: app-private sandbox dir; desktop keeps dirs::config_dir.
        let mut path = match crate::services::app_paths::mobile_base() {
            Some(base) => base.join("StreamNook"),
            None => {
                let mut p = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
                p.push("StreamNook");
                p
            }
        };
        if !path.exists() {
            fs::create_dir_all(&path)?;
        }
        path.push(format!(".seventv_token_{}", twitch_id));
        Ok(path)
    }

    fn load_token_for(twitch_id: &str) -> Result<StorableSevenTVToken> {
        let path = Self::account_token_file_path(twitch_id)?;
        if !path.exists() {
            return Err(anyhow::anyhow!("No 7TV token for account {}", twitch_id));
        }
        let bytes = fs::read(&path)?;
        let decoded = Self::xor_obfuscate(&bytes);
        let s = String::from_utf8(decoded)
            .map_err(|_| anyhow::anyhow!("Corrupt 7TV token for account {}", twitch_id))?;
        let token: StorableSevenTVToken = serde_json::from_str(&s)?;
        Ok(token)
    }

    /// Store a linked account's 7TV token (after the incognito login captures it).
    pub async fn store_token_for(
        twitch_id: &str,
        access_token: String,
        seventv_user_id: String,
    ) -> Result<()> {
        let token = StorableSevenTVToken {
            access_token,
            user_id: seventv_user_id,
            twitch_id: twitch_id.to_string(),
            created_at: chrono::Utc::now().timestamp(),
        };
        let json = serde_json::to_string(&token)?;
        let path = Self::account_token_file_path(twitch_id)?;
        fs::write(&path, Self::xor_obfuscate(json.as_bytes()))?;
        debug!("[7TV_AUTH] stored 7TV token for account {}", twitch_id);
        Ok(())
    }

    /// A linked account's 7TV access token, if connected.
    pub async fn get_token_for(twitch_id: &str) -> Result<String> {
        Self::load_token_for(twitch_id).map(|t| t.access_token)
    }

    /// Auth status for a linked account (instant JWT-exp check, no network).
    pub async fn get_auth_status_for(twitch_id: &str) -> SevenTVAuthStatus {
        match Self::load_token_for(twitch_id) {
            Ok(token) => {
                if Self::is_token_expired(&token.access_token) {
                    let _ = Self::logout_for(twitch_id).await;
                    SevenTVAuthStatus {
                        is_authenticated: false,
                        user_id: None,
                        twitch_id: None,
                    }
                } else {
                    SevenTVAuthStatus {
                        is_authenticated: true,
                        user_id: Some(token.user_id),
                        twitch_id: Some(token.twitch_id),
                    }
                }
            }
            Err(_) => SevenTVAuthStatus {
                is_authenticated: false,
                user_id: None,
                twitch_id: None,
            },
        }
    }

    /// Disconnect a linked account's 7TV (delete its stored token).
    pub async fn logout_for(twitch_id: &str) -> Result<()> {
        let path = Self::account_token_file_path(twitch_id)?;
        if path.exists() {
            fs::remove_file(&path)?;
            debug!("[7TV_AUTH] 7TV token deleted for account {}", twitch_id);
        }
        Ok(())
    }

    /// Authoritatively check a linked account's 7TV token against 7TV (not just
    /// the local JWT-exp heuristic). Clears the stored token and returns false if
    /// 7TV rejects it, so a revoked/dead token can't keep reading as "connected".
    pub async fn validate_token_for(twitch_id: &str) -> Result<bool> {
        let token = match Self::get_token_for(twitch_id).await {
            Ok(t) => t,
            Err(_) => return Ok(false),
        };

        let client = crate::services::http::client().clone();
        let response = client
            .post(SEVENTV_GQL_URL)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token))
            .json(&serde_json::json!({ "query": "{ users { me { id } } }" }))
            .send()
            .await?;

        if !response.status().is_success() {
            let _ = Self::logout_for(twitch_id).await;
            return Ok(false);
        }

        let json: serde_json::Value = response.json().await?;
        if json["data"]["users"]["me"]["id"].is_string() {
            Ok(true)
        } else {
            let _ = Self::logout_for(twitch_id).await;
            Ok(false)
        }
    }
}

/// 7TV Cosmetics Service - uses the auth token to change paints/badges
pub struct SevenTVCosmeticsService;

impl SevenTVCosmeticsService {
    /// Authenticated POST to the 7TV GQL API returning the full parsed JSON body
    /// (data plus any non-auth errors). On an AUTH failure (HTTP 401 or a
    /// `LOGIN_REQUIRED` GraphQL error) it clears the relevant stored token
    /// (`cleanup = Some(twitch_id)` for a linked account, `None` for the primary)
    /// and returns SESSION_EXPIRED. Non-auth GraphQL errors are left in the
    /// returned JSON for the caller to surface (e.g. set full, name conflict).
    pub async fn post_authed(
        token: &str,
        cleanup: Option<&str>,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let client = crate::services::http::client().clone();

        let response = client
            .post(SEVENTV_GQL_URL)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await?;
            if status == reqwest::StatusCode::UNAUTHORIZED {
                debug!("[7TV] authed request returned 401 - clearing token");
                Self::clear_token(cleanup).await;
                return Err(anyhow::anyhow!(
                    "SESSION_EXPIRED: 7TV session has expired. Please reconnect your 7TV account."
                ));
            }
            return Err(anyhow::anyhow!("Failed 7TV request: {}", error_text));
        }

        let result: serde_json::Value = response.json().await?;

        // Auth-class GraphQL errors clear the token and surface SESSION_EXPIRED;
        // every other error stays in the payload for the caller to handle.
        if let Some(arr) = result.get("errors").and_then(|e| e.as_array()) {
            let auth_err = arr.iter().any(|err| {
                err.get("extensions")
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
                    .map(|c| c == "LOGIN_REQUIRED")
                    .unwrap_or(false)
            });
            if auth_err {
                debug!("[7TV] GQL returned LOGIN_REQUIRED - clearing token");
                Self::clear_token(cleanup).await;
                return Err(anyhow::anyhow!(
                    "SESSION_EXPIRED: 7TV session has expired. Please reconnect your 7TV account."
                ));
            }
        }

        Ok(result)
    }

    /// Run a cosmetics mutation with an explicit token. Builds the request body,
    /// delegates the authed POST + token-cleanup to `post_authed`, and collapses
    /// any GraphQL error into an `Err` (the paint/badge callers only need
    /// success/failure).
    async fn run_mutation(
        token: &str,
        cleanup: Option<&str>,
        query: &str,
        variables: serde_json::Value,
        op_name: &str,
    ) -> Result<bool> {
        let body = serde_json::json!({
            "query": query,
            "variables": variables,
            "operationName": op_name,
        });
        let result = Self::post_authed(token, cleanup, body).await?;

        if let Some(arr) = result.get("errors").and_then(|e| e.as_array()) {
            if !arr.is_empty() {
                return Err(anyhow::anyhow!("GraphQL errors: {:?}", arr));
            }
        }

        Ok(true)
    }

    async fn clear_token(cleanup: Option<&str>) {
        match cleanup {
            Some(twitch_id) => {
                let _ = SevenTVAuthService::logout_for(twitch_id).await;
            }
            None => {
                let _ = SevenTVAuthService::logout().await;
            }
        }
    }

    const PAINT_MUTATION: &'static str = r#"
        mutation SetActivePaint($id: Id!, $paintId: Id) {
            users { user(id: $id) { activePaint(paintId: $paintId) { id style { activePaintId } } } }
        }
    "#;

    const BADGE_MUTATION: &'static str = r#"
        mutation SetActiveBadge($id: Id!, $badgeId: Id) {
            users { user(id: $id) { activeBadge(badgeId: $badgeId) { id style { activeBadgeId } } } }
        }
    "#;

    /// Set the primary's active paint.
    pub async fn set_active_paint(user_id: &str, paint_id: Option<&str>) -> Result<bool> {
        let token = SevenTVAuthService::get_token().await?;
        Self::run_mutation(
            &token,
            None,
            Self::PAINT_MUTATION,
            serde_json::json!({ "id": user_id, "paintId": paint_id }),
            "SetActivePaint",
        )
        .await
    }

    /// Set a linked account's active paint, using that account's 7TV token.
    pub async fn set_active_paint_for(
        twitch_id: &str,
        user_id: &str,
        paint_id: Option<&str>,
    ) -> Result<bool> {
        let token = SevenTVAuthService::get_token_for(twitch_id).await?;
        Self::run_mutation(
            &token,
            Some(twitch_id),
            Self::PAINT_MUTATION,
            serde_json::json!({ "id": user_id, "paintId": paint_id }),
            "SetActivePaint",
        )
        .await
    }

    /// Set the primary's active badge.
    pub async fn set_active_badge(user_id: &str, badge_id: Option<&str>) -> Result<bool> {
        let token = SevenTVAuthService::get_token().await?;
        Self::run_mutation(
            &token,
            None,
            Self::BADGE_MUTATION,
            serde_json::json!({ "id": user_id, "badgeId": badge_id }),
            "SetActiveBadge",
        )
        .await
    }

    /// Set a linked account's active badge, using that account's 7TV token.
    pub async fn set_active_badge_for(
        twitch_id: &str,
        user_id: &str,
        badge_id: Option<&str>,
    ) -> Result<bool> {
        let token = SevenTVAuthService::get_token_for(twitch_id).await?;
        Self::run_mutation(
            &token,
            Some(twitch_id),
            Self::BADGE_MUTATION,
            serde_json::json!({ "id": user_id, "badgeId": badge_id }),
            "SetActiveBadge",
        )
        .await
    }
}
