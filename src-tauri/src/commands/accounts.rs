//! Tauri commands for the multi-account registry. Phase 1 exposes only the
//! read side (list + count) that later phases and the "send as" picker build on.
//! Add / remove / set-primary commands arrive with the add-account flow.

use crate::models::settings::AppState;
use crate::services::account_store::{AccountStore, StoredAccount};
use crate::services::token_vault::{self, CredentialStorage};
use crate::services::twitch_service::TwitchService;
use crate::utils::oauth_server;
use std::time::Duration;
use crate::rt::AppHandle;
use tauri::State;

/// The accounts a sign-out takes off the automations: the registry's main,
/// plus the profile's account only when `list()` does not name it as a linked
/// account (right after a switch the profile can still name the previous main,
/// now a staying alt, which is left alone).
pub(crate) fn sign_out_hook_ids() -> Vec<String> {
    // One read, so the main and the linked set come from the same registry.
    let list = AccountStore::list();
    let main = list.iter().find(|a| a.is_primary).map(|a| a.user_id.as_str());
    let linked: Vec<String> = list.iter().filter(|a| !a.is_primary).map(|a| a.user_id.clone()).collect();
    let profile = crate::services::account_roster::twitch_user_id();
    crate::services::account_automations::sign_out_ids(main, profile.as_deref(), &linked)
}

/// Every account a forced re-auth removes: all registered ids plus the
/// sign-out set, distinct.
pub(crate) fn reauth_hook_ids() -> Vec<String> {
    let mut ids: Vec<String> = AccountStore::list().into_iter().map(|a| a.user_id).collect();
    for id in sign_out_hook_ids() {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// Whether removing `id` should hold its rules first: always, unless the
/// registry shows `id` as the main (the door refuses that itself).
fn remove_hook_wanted(id: &str) -> bool {
    let pairs: Vec<(String, bool)> = AccountStore::list().into_iter().map(|a| (a.user_id, a.is_primary)).collect();
    crate::services::account_automations::remove_needs_hook(&pairs, id)
}

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

/// Whether stored sign-ins are protected by the system keyring, for the notice
/// under Accounts. Off the async runtime because asking the keyring can wait on
/// a prompt.
#[tauri::command]
pub async fn get_credential_storage() -> Result<CredentialStorage, String> {
    tauri::async_runtime::spawn_blocking(token_vault::credential_storage)
        .await
        .map_err(|e| e.to_string())
}

/// Link a NEW secondary account via the system browser.
///
/// Forces Twitch's account chooser (`force_verify=true`) in the user's default
/// browser, captures the redirect on the localhost callback, exchanges the code
/// for that account's own token, and files it as a SECONDARY. This never reads
/// or writes the primary's cached token, and `add_secondary` rejects the attempt
/// if the chosen account is already the primary. Resolves with the linked
/// account, or an error string the UI can surface (cancelled, timed out, etc.).
#[cfg(not(target_os = "ios"))]
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

#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn add_twitch_account(_app: AppHandle) -> Result<StoredAccount, String> {
    Err(crate::ios_ffi::not_wired("Linking an account in the browser"))
}

/// Unlink a secondary account (removes its stored token and registry entry).
/// The primary cannot be removed here.
#[tauri::command]
pub async fn remove_twitch_account(user_id: String) -> Result<(), String> {
    // Rules sending as this account stop and stay held until the door returns,
    // unless a clean read shows it is the main (the door refuses that itself).
    let _leaving = if remove_hook_wanted(&user_id) {
        Some(crate::services::account_automations::before_account_removed(&user_id).await)
    } else {
        None
    };
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
    // No alt refresh starts, and none is in flight, while the main moves. The
    // guard lives for the `set_active` call only, not for the work after it.
    let result = {
        let _change = AccountStore::begin_account_change().await;
        AccountStore::set_active(&user_id)
            .await
            .map_err(|e| e.to_string())
    };
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
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<StoredAccount>, String> {
    // Both the registry's main and the slot's account can leave; one guard
    // holds their rules until this door returns.
    let _leaving = crate::services::account_automations::before_accounts_removed(&sign_out_hook_ids()).await;
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
        // The phone's sign-in browser still holds the outgoing account's
        // twitch.tv session, which the web-session reads and writes ride. Wiped
        // on every sign-out, promoted or full, so nothing acts as the account
        // that left and the next sign-in starts at Twitch's login page.
        #[cfg(target_os = "android")]
        if let Err(e) = crate::twitch_login_plugin::clear_mobile_login_cookies(app).await {
            log::warn!("[accounts] sign-out could not clear the sign-in browser: {e}");
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = app;
    result
}

#[cfg(test)]
mod automation_hook_tests {
    fn body(sig: &str) -> String {
        let src = include_str!("accounts.rs").replace("\r\n", "\n");
        let start = src.find(sig).unwrap_or_else(|| panic!("{sig}"));
        src[start..start + src[start..].find("\n}\n").unwrap()].to_string()
    }

    #[test]
    fn the_doors_hold_their_guard_to_the_end() {
        let remove = body("pub async fn remove_twitch_account(");
        assert!(remove.contains("let _leaving = ") && remove.contains("before_account_removed(&user_id)"));
        assert!(remove.find("let _leaving = ").unwrap() < remove.find("remove_secondary").unwrap());
        let set_active = body("pub async fn set_active_twitch_account(");
        // The guard is bound inside a block that ends with the `set_active` call.
        let block = set_active.find("let result = {
        let _change = AccountStore::begin_account_change().await;").expect("a scoped guard");
        let call = set_active.find("AccountStore::set_active(").unwrap();
        assert!(block < call && set_active[call..].find("
    };
").is_some(), "the block closes after the call");
        assert!(set_active[call..].find("state.twitch_auth").unwrap() > set_active[call..].find("
    };
").unwrap(), "the work after it runs without the guard");
        let sign_out = body("pub async fn sign_out_active_twitch_account(");
        assert!(sign_out.contains("let _leaving = ") && sign_out.contains("sign_out_hook_ids()"));
        assert!(sign_out.find("let _leaving = ").unwrap() < sign_out.find("sign_out_active()").unwrap());
        let logout = include_str!("twitch.rs").replace("\r\n", "\n");
        let start = logout.find("pub async fn twitch_logout(").unwrap();
        let logout = &logout[start..start + logout[start..].find("\n}\n").unwrap()];
        assert!(logout.contains("let _leaving = ") && logout.contains("sign_out_hook_ids()"));
        assert!(logout.find("let _leaving = ").unwrap() < logout.find("TwitchService::logout()").unwrap());
        let svc = include_str!("../services/twitch_service.rs").replace("\r\n", "\n");
        let at = svc.find("AccountStore::reset_all().await;").unwrap();
        let before = &svc[at.saturating_sub(500)..at];
        assert!(before.contains("let _leaving = ") && before.contains("before_accounts_removed("));
    }
}
