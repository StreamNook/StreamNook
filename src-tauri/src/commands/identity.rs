//! StreamNook Identity API client.
//!
//! Thin Rust commands over the public Identity API on streamnook.app
//! (`/api/v1/identity`). Networking lives in Rust because the write path needs
//! the user's Twitch access token as a bearer (the token is intentionally not
//! exposed to the webview), and keeping reads here too means no Tauri HTTP
//! capability allowlisting and no CORS to reason about — same posture as the
//! badge/cosmetics services.
//!
//! The loadout is a member's chosen subset of aggregated badges to display as
//! their StreamNook presence. Reads are public; the write is owner-verified
//! server-side (the API validates the bearer token against Twitch before it
//! upserts the row).

use crate::services::account_store::AccountStore;
use crate::services::twitch_service::TwitchService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const IDENTITY_API: &str = "https://streamnook.app/api/v1/identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityLoadout {
    pub twitch_user_id: String,
    pub customized: bool,
    pub badges: Vec<String>,
    pub paint: Option<String>,
    pub updated_at: Option<String>,
}

fn default_loadout(user_id: &str) -> IdentityLoadout {
    IdentityLoadout {
        twitch_user_id: user_id.to_string(),
        customized: false,
        badges: Vec::new(),
        paint: None,
        updated_at: None,
    }
}

#[derive(Debug, Deserialize)]
struct BatchResponse {
    identities: HashMap<String, IdentityLoadout>,
}

#[derive(Debug, Serialize)]
struct PutBody {
    badges: Vec<String>,
    paint: Option<String>,
    customized: bool,
}

/// Read a single user's applied identity. Falls back to the non-customized
/// default (show-all) on any error so a backend hiccup never breaks rendering.
#[tauri::command]
pub async fn get_streamnook_identity(user_id: String) -> Result<IdentityLoadout, String> {
    let client = crate::services::http::client();
    let url = format!("{}/{}", IDENTITY_API, user_id);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<IdentityLoadout>()
            .await
            .or_else(|_| Ok(default_loadout(&user_id))),
        _ => Ok(default_loadout(&user_id)),
    }
}

/// Batch read for chat backfill. Returns a map keyed by twitch_user_id, with a
/// non-customized default for any id the backend didn't return.
#[tauri::command]
pub async fn get_streamnook_identities(
    user_ids: Vec<String>,
) -> Result<HashMap<String, IdentityLoadout>, String> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let client = crate::services::http::client();
    let ids = user_ids.join(",");
    let result = client
        .get(IDENTITY_API)
        .query(&[("ids", ids.as_str())])
        .send()
        .await;

    let mut map = match result {
        Ok(resp) if resp.status().is_success() => match resp.json::<BatchResponse>().await {
            Ok(body) => body.identities,
            Err(_) => HashMap::new(),
        },
        _ => HashMap::new(),
    };

    for id in &user_ids {
        map.entry(id.clone()).or_insert_with(|| default_loadout(id));
    }
    Ok(map)
}

/// What `set_streamnook_identity` returns: the saved loadout (flattened to the
/// same shape `IdentityLoadout` had) plus the authoritative resolved bundle the
/// server computed for the just-saved selection. Returning the resolved bundle
/// here lets the client populate its render cache directly instead of re-reading
/// the `?resolve=1` endpoint, which is edge-cached ~60s and would otherwise serve
/// the pre-write value and make a fresh change look unsaved. `resolved` is
/// optional so an older backend that doesn't send it degrades gracefully.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetIdentityResult {
    #[serde(flatten)]
    pub loadout: IdentityLoadout,
    #[serde(default)]
    pub resolved: Option<ResolvedIdentity>,
}

/// Write the current user's loadout. The server validates the bearer token
/// against Twitch and only writes the row it resolves to, so a client can never
/// set another user's identity.
#[tauri::command]
pub async fn set_streamnook_identity(
    badges: Vec<String>,
    paint: Option<String>,
    customized: bool,
    account_id: Option<String>,
) -> Result<SetIdentityResult, String> {
    // Authenticate the write as the chosen account when one is given (a linked
    // secondary), else the primary. The server upserts whichever account the
    // bearer token resolves to, so each account can only write its own row.
    let token = match account_id {
        Some(id) => AccountStore::get_token_for(&id)
            .await
            .map_err(|e| format!("No token for account {}: {}", id, e))?,
        None => TwitchService::get_token()
            .await
            .map_err(|e| format!("No Twitch token: {}", e))?,
    };

    let client = crate::services::http::client();
    let resp = client
        .put(IDENTITY_API)
        .bearer_auth(&token)
        .json(&PutBody {
            badges,
            paint,
            customized,
        })
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Identity write failed ({}): {}", status, body));
    }

    let mut result = resp
        .json::<SetIdentityResult>()
        .await
        .map_err(|e| format!("Bad response: {}", e))?;
    result.resolved = result.resolved.map(ResolvedIdentity::in_provider_order);
    Ok(result)
}

// ── Resolved (all-in-one) identity ──────────────────────────────────────────
// The ?resolve=1 bundle: the member's selected third-party badges already
// resolved to images (server-side, ownership-checked) plus their live 7TV
// badge/paint. Chat fetches this once per StreamNook user instead of resolving
// every provider locally. Twitch badges are not included (they ride the IRC tags).

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedBadge {
    pub key: String,
    pub provider: String,
    pub title: String,
    pub image_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedIdentity {
    pub twitch_user_id: String,
    pub customized: bool,
    pub badges: Vec<ResolvedBadge>,
    // 7TV badge + paint passed through verbatim so the frontend can render the
    // paint with its existing helper without us coupling to the paint schema.
    #[serde(default)]
    pub seventv: serde_json::Value,
    pub updated_at: Option<String>,
}

impl ResolvedIdentity {
    /// The chosen badges in the fixed provider order, whatever order the server
    /// sent (older deployments sent them in the order they were switched on).
    /// Stable, so badges of one provider keep the server's feed order.
    fn in_provider_order(mut self) -> Self {
        self.badges.sort_by_key(|b| crate::services::badge_service::third_party_rank(&b.provider));
        self
    }
}

fn default_resolved(user_id: &str) -> ResolvedIdentity {
    ResolvedIdentity {
        twitch_user_id: user_id.to_string(),
        customized: false,
        badges: Vec::new(),
        seventv: serde_json::Value::Null,
        updated_at: None,
    }
}

/// Fetch the resolved all-in-one identity bundle for a user. Falls back to the
/// non-customized default on any error so chat never breaks over a hiccup.
#[tauri::command]
pub async fn get_streamnook_identity_resolved(user_id: String) -> Result<ResolvedIdentity, String> {
    let client = crate::services::http::client();
    let url = format!("{}/{}?resolve=1", IDENTITY_API, user_id);
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => Ok(resp
            .json::<ResolvedIdentity>()
            .await
            .map(ResolvedIdentity::in_provider_order)
            .unwrap_or_else(|_| default_resolved(&user_id))),
        _ => Ok(default_resolved(&user_id)),
    }
}
