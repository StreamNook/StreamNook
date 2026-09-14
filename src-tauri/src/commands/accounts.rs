//! Tauri commands for the multi-account registry. Phase 1 exposes only the
//! read side (list + count) that later phases and the "send as" picker build on.
//! Add / remove / set-primary commands arrive with the add-account flow.

use crate::models::settings::AppState;
use crate::services::account_store::{AccountStore, StoredAccount};
use crate::services::twitch_service::TwitchService;
use crate::utils::oauth_server;
use std::time::Duration;
use tauri::{AppHandle, State};

/// Every linked account, primary first. The frontend uses this to render the
/// account list and the "send as" picker.
#[tauri::command]
pub async fn list_twitch_accounts() -> Result<Vec<StoredAccount>, String> {
    Ok(AccountStore::list())
}

/// Number of linked accounts. The "send as" picker is only shown when this is
/// 2 or more, so a single-account user sees no change.
#[tauri::command]
pub async fn get_twitch_account_count() -> Result<usize, String> {
    Ok(AccountStore::count())
}

/// Link a NEW secondary account via the system browser.
///
/// Forces Twitch's account chooser (`force_verify=true`) in the user's default
/// browser, captures the redirect on the localhost callback, exchanges the code
/// for that account's own token, and files it as a SECONDARY. This never reads
/// or writes the primary's cached token, and `add_secondary` rejects the attempt
/// if the chosen account is already the primary. Resolves with the linked
/// account, or an error string the UI can surface (cancelled, timed out, etc.).
#[tauri::command]
pub async fn add_twitch_account(app: AppHandle) -> Result<StoredAccount, String> {
    use tauri_plugin_opener::OpenerExt;

    // Opaque CSRF token, verified against the redirect's `state`.
    let state = format!("{:032x}", rand::random::<u128>());

    // Bind the callback listener BEFORE opening the browser so a fast redirect
    // can't arrive before we're ready.
    let listener = oauth_server::start_oauth_listener()
        .await
        .map_err(|e| e.to_string())?;

    let url = TwitchService::build_authorize_url(&state).map_err(|e| e.to_string())?;
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|e| format!("Failed to open browser: {}", e))?;

    // Wait up to five minutes for the user to finish signing in.
    let callback = listener
        .wait(Duration::from_secs(300))
        .await
        .map_err(|e| e.to_string())?;

    if let Some(err) = callback.error {
        return Err(format!("Sign-in was cancelled or failed: {}", err));
    }
    if callback.state.as_deref() != Some(state.as_str()) {
        return Err(
            "Sign-in could not be verified (state mismatch). Please try again.".to_string(),
        );
    }
    if callback.code.is_empty() {
        return Err("Twitch did not return an authorization code.".to_string());
    }

    let token = TwitchService::exchange_code_for_token(&callback.code)
        .await
        .map_err(|e| e.to_string())?;

    AccountStore::add_secondary(token)
        .await
        .map_err(|e| e.to_string())
}

/// Unlink a secondary account (removes its stored token and registry entry).
/// The primary cannot be removed here.
#[tauri::command]
pub async fn remove_twitch_account(user_id: String) -> Result<(), String> {
    AccountStore::remove_secondary(&user_id).map_err(|e| e.to_string())
}

/// Promote a linked account to the main account you watch and stream as. Moves
/// its token into the primary slot and demotes the previous main to a linked
/// account (kept, never deleted). Returns the newly-active account. The frontend
/// re-establishes the watched identity (and reconnects chat) afterward.
#[tauri::command]
pub async fn set_active_twitch_account(
    user_id: String,
    state: State<'_, AppState>,
) -> Result<StoredAccount, String> {
    let result = AccountStore::set_active(&user_id)
        .await
        .map_err(|e| e.to_string());
    if result.is_ok() {
        // The active account changed: re-harvest the web session for the new
        // account on the next resolve, and drop any Turbo/sub verdict cached
        // against the previous account's token.
        state.twitch_auth.on_account_changed().await;
        crate::services::auth_proxy::clear_entitlement_caches();
        // GIF eligibility is per ACCOUNT (Twitch's server-side Tier 2/3 gate),
        // so a cached "allowlisted" would otherwise follow the old account.
        crate::commands::gifs::clear_config_cache().await;
        // The drops/points credential belongs to the previous account; clear it
        // so the heartbeat doesn't keep crediting the account you switched away
        // from. The new account re-authorizes drops separately when wanted.
        let _ = crate::services::drops_auth_service::DropsAuthService::logout().await;
    }
    result
}

/// Sign out of the current main. If other accounts are linked, the most recently
/// added one is promoted to main and returned; otherwise this is a full sign-out
/// and returns `null`. The frontend re-establishes identity when an account is
/// returned, or drops to the logged-out state when it is null.
#[tauri::command]
pub async fn sign_out_active_twitch_account(
    state: State<'_, AppState>,
) -> Result<Option<StoredAccount>, String> {
    let result = AccountStore::sign_out_active()
        .await
        .map_err(|e| e.to_string());
    match &result {
        // Another linked account was promoted into the main slot.
        Ok(Some(_)) => state.twitch_auth.on_account_changed().await,
        // Full sign-out: reads stay logged-out until the next login.
        Ok(None) => state.twitch_auth.on_logged_out().await,
        Err(_) => {}
    }
    if result.is_ok() {
        crate::services::auth_proxy::clear_entitlement_caches();
        crate::commands::gifs::clear_config_cache().await;
        // The outgoing account's drops/points credential is now wrong whether we
        // promoted another account or fully signed out; clear it.
        let _ = crate::services::drops_auth_service::DropsAuthService::logout().await;
    }
    result
}
