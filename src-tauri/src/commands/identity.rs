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
use crate::services::shared_lookup::{Lookup, SharedLookup};
use crate::services::twitch_service::TwitchService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

const IDENTITY_API: &str = "https://streamnook.app/api/v1/identity";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// What the Identity API answers: the body, plus `fallback` when the server
/// could not read the member and sent the default in its place.
#[derive(Deserialize)]
struct IdentityAnswer<T> {
    #[serde(flatten)]
    body: T,
    #[serde(default)]
    fallback: bool,
}

/// One answer of the Identity API, parsed. The default body is also what a
/// member who chose nothing gets, so a failure passed off as it would blank
/// badges that are really worn: the server's own fallback and a body that does
/// not parse are errors, never the default.
fn parse_identity_answer<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    let answer: IdentityAnswer<T> =
        serde_json::from_str(text).map_err(|e| format!("identity answer did not parse: {e}"))?;
    if answer.fallback {
        return Err("identity server could not read the loadout".to_string());
    }
    Ok(answer.body)
}

/// One Identity API read: an error when nothing answered, the status was not a
/// success, or the answer is not a real one (see `parse_identity_answer`).
/// Callers that only render chat fall back on their side; the profile card
/// tells a failed read from a member who chose nothing by this.
async fn read_identity<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let client = crate::services::http::client();
    let resp = crate::services::sn_transport::send(client.get(url))
        .await
        .map_err(|e| format!("identity read failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("identity read answered {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| format!("identity read failed: {e}"))?;
    parse_identity_answer(&text)
}

/// A loadout read within this long is served as it was; a member's own save
/// replaces it at once (`set_streamnook_identity` primes it).
const LOADOUT_TTL: Duration = Duration::from_secs(60);
/// The longest the profile sheet and the You header wait on a loadout read. The
/// read itself goes on past this and fills the cache.
pub const LOADOUT_BUDGET: Duration = Duration::from_millis(1500);
const LOADOUT_CAP: usize = 512;

/// Every loadout read for a profile surface: one read per member at a time, in a
/// task that outlives its callers, errors never kept, the newest write winning.
static LOADOUTS: LazyLock<SharedLookup<IdentityLoadout>> = LazyLock::new(|| SharedLookup::new(LOADOUT_CAP));

/// A member's loadout for a profile surface, waiting at most `LOADOUT_BUDGET`.
/// `Ok(None)` is a budget miss with nothing kept: the part is unknown, not empty.
/// An unreachable server with nothing kept is an `Err`; with an older copy it is
/// that copy.
pub async fn identity_loadout(member_id: &str) -> Result<Option<IdentityLoadout>, String> {
    identity_loadout_within(member_id, LOADOUT_BUDGET).await
}

/// `identity_loadout` with the caller's own wait.
pub async fn identity_loadout_within(member_id: &str, budget: Duration) -> Result<Option<IdentityLoadout>, String> {
    if member_id.is_empty() {
        return Ok(None);
    }
    let url = format!("{}/{}", IDENTITY_API, member_id);
    match LOADOUTS
        .lookup(member_id, LOADOUT_TTL, budget, move || async move { read_identity::<IdentityLoadout>(&url).await })
        .await
    {
        Lookup::Fresh(loadout) | Lookup::Stale(loadout) => Ok(Some(loadout)),
        Lookup::Pending => Ok(None),
        Lookup::Failed(e) => Err(e),
    }
}

/// The number of the write that holds a member's cached loadout (0 when none
/// does). A rebuild reads it after its loadout read and writes its result only
/// while it is unchanged, so a save that landed meanwhile keeps the last word.
pub fn loadout_stamp(member_id: &str) -> u64 {
    LOADOUTS.stamp(member_id)
}

/// Read a single user's applied identity. A failure is an error (the page
/// falls back to show-all and marks the stand-in as a failed read).
#[tauri::command]
pub async fn get_streamnook_identity(user_id: String) -> Result<IdentityLoadout, String> {
    read_identity(&format!("{}/{}", IDENTITY_API, user_id)).await
}

/// A member's profile card for its first paint, before the live sources answer:
/// the card streamnook.app wrote for them, with every link in it checked again
/// here, or just their identity when there is no usable row. Also asks the
/// server to rebuild a stale card. None for a non-member, a malformed id, or
/// any failure.
#[tauri::command]
pub async fn get_profile_snapshot(user_id: String) -> Option<crate::services::profile_snapshot::FirstPaint> {
    crate::services::profile_snapshot::first_paint(&user_id).await
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
    let result =
        crate::services::sn_transport::send(client.get(IDENTITY_API).query(&[("ids", ids.as_str())])).await;

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
    // The member this save is for: the chosen account, else the primary.
    let acting = account_id.clone().or_else(crate::services::account_roster::twitch_user_id);
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
    let request = client.put(IDENTITY_API).bearer_auth(&token).json(&PutBody {
        badges,
        paint,
        customized,
    });
    let resp = crate::services::sn_transport::send(request)
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
    // Every phone surface now reads what was just saved: the loadout cache holds
    // it (a read that began before the save loses to it), the member's sheets
    // are built again, and the You header file is rebuilt, never deleted.
    if let Some(member) = acting.filter(|m| !m.is_empty()) {
        LOADOUTS.prime(&member, result.loadout.clone());
        crate::services::profile_view::bump_member_epoch(&member);
        tauri::async_runtime::spawn(async move {
            crate::services::profile_view::rebuild_own_header(&member).await;
        });
    }
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


/// Fetch the resolved all-in-one identity bundle for a user. A failure is an
/// error: chat falls back to the default on its side, and the profile card does
/// not take it for a member who wears nothing.
#[tauri::command]
pub async fn get_streamnook_identity_resolved(user_id: String) -> Result<ResolvedIdentity, String> {
    read_identity::<ResolvedIdentity>(&format!("{}/{}?resolve=1", IDENTITY_API, user_id))
        .await
        .map(ResolvedIdentity::in_provider_order)
}

#[cfg(test)]
mod identity_answer_tests {
    use super::*;

    const NONE_CHOSEN: &str =
        r#"{"twitch_user_id":"5","customized":false,"badges":[],"paint":null,"updated_at":null}"#;

    #[test]
    fn a_member_who_chose_nothing_is_an_answer() {
        let loadout: IdentityLoadout = parse_identity_answer(NONE_CHOSEN).unwrap();
        assert!(!loadout.customized && loadout.badges.is_empty());
    }

    #[test]
    fn the_servers_fallback_is_an_error_not_the_default() {
        let fallback = NONE_CHOSEN.replace("\"updated_at\":null", "\"updated_at\":null,\"fallback\":true");
        assert!(parse_identity_answer::<IdentityLoadout>(&fallback).is_err());
        let resolved = r#"{"twitch_user_id":"5","customized":false,"badges":[],"seventv":{"badge":null,"paint":null},"updated_at":null,"fallback":true}"#;
        assert!(parse_identity_answer::<ResolvedIdentity>(resolved).is_err());
    }

    #[test]
    fn a_body_that_does_not_parse_is_an_error() {
        assert!(parse_identity_answer::<IdentityLoadout>("<html>502</html>").is_err());
        assert!(parse_identity_answer::<ResolvedIdentity>(r#"{"badges":"nope"}"#).is_err());
    }

    #[test]
    fn a_resolved_answer_keeps_its_badges_and_ignores_unknown_fields() {
        let text = r#"{"twitch_user_id":"5","customized":true,"badges":[{"key":"ffz:ffz-3","provider":"ffz","title":"FFZ","image_url":"https://cdn.frankerfacez.com/badge/3/4/rounded"}],"seventv":null,"updated_at":"2026-10-30","later":1,"fallback":false}"#;
        let resolved: ResolvedIdentity = parse_identity_answer(text).unwrap();
        assert_eq!(resolved.badges.len(), 1);
        assert_eq!(resolved.badges[0].provider, "ffz");
    }
}

#[cfg(test)]
mod loadout_tests {
    use super::*;

    fn saved(id: &str, badges: &[&str]) -> IdentityLoadout {
        IdentityLoadout {
            twitch_user_id: id.to_string(),
            customized: true,
            badges: badges.iter().map(|b| b.to_string()).collect(),
            paint: None,
            updated_at: None,
        }
    }

    #[tokio::test]
    async fn a_saved_loadout_is_served_from_the_cache_without_a_read() {
        LOADOUTS.prime("test-loadout-1", saved("test-loadout-1", &["ffz:ffz-3"]));
        let got = identity_loadout("test-loadout-1").await.unwrap().expect("the primed loadout");
        assert!(got.customized);
        assert_eq!(got.badges, vec!["ffz:ffz-3".to_string()]);
    }

    #[tokio::test]
    async fn an_empty_id_asks_nothing_and_a_save_moves_the_stamp() {
        assert!(matches!(identity_loadout("").await, Ok(None)));
        assert_eq!(loadout_stamp("test-loadout-never"), 0);
        LOADOUTS.prime("test-loadout-2", saved("test-loadout-2", &[]));
        let first = loadout_stamp("test-loadout-2");
        assert!(first > 0);
        LOADOUTS.prime("test-loadout-2", saved("test-loadout-2", &["bttv:x"]));
        assert!(loadout_stamp("test-loadout-2") > first, "a newer save is a newer stamp");
    }
}
