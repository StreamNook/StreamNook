//! FFZ account status for the LOCAL user. Gates which FFZ effect emotes the
//! picker and tab-complete OFFER (subscriber-only effects need an FFZ
//! subscription); rendering of effects in incoming messages is never gated.

use serde::Serialize;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::services::account_store::AccountStore;

#[derive(Clone, Serialize)]
pub struct FfzUserStatus {
    pub is_subwoofer: bool,
}

const TTL: Duration = Duration::from_secs(3600);

// Cached per login so switching accounts naturally invalidates. A negative
// answer (no FFZ account / 404 / error) is cached too, otherwise every picker
// open for a non-FFZ user would re-fire the lookup.
static CACHE: OnceLock<RwLock<Option<(String, Instant, bool)>>> = OnceLock::new();

fn cache() -> &'static RwLock<Option<(String, Instant, bool)>> {
    CACHE.get_or_init(|| RwLock::new(None))
}

/// The local user's FFZ subscriber ("subwoofer") status, cached ~1h.
#[tauri::command]
pub async fn ffz_local_user_status() -> FfzUserStatus {
    let Some(account) = AccountStore::primary() else {
        return FfzUserStatus {
            is_subwoofer: false,
        };
    };
    let login = account.login.to_lowercase();

    if let Some((cached_login, at, val)) = cache().read().await.clone() {
        if cached_login == login && at.elapsed() < TTL {
            return FfzUserStatus { is_subwoofer: val };
        }
    }

    let is_subwoofer = fetch_is_subwoofer(&login).await;
    *cache().write().await = Some((login, Instant::now(), is_subwoofer));
    FfzUserStatus { is_subwoofer }
}

/// The cached answer for the signed-in account, without a lookup. False until
/// `ffz_local_user_status` has run, which is also its answer when unsure.
pub async fn cached_is_subwoofer() -> bool {
    let Some(account) = AccountStore::primary() else {
        return false;
    };
    let login = account.login.to_lowercase();
    matches!(cache().read().await.as_ref(), Some((l, _, true)) if *l == login)
}

async fn fetch_is_subwoofer(login: &str) -> bool {
    let url = format!("https://api.frankerfacez.com/v1/user/{}", login);
    match crate::services::http::client().get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.pointer("/user/is_subwoofer").and_then(|b| b.as_bool()))
            .unwrap_or(false),
        _ => false, // 404 = no FFZ account; errors gate closed
    }
}
