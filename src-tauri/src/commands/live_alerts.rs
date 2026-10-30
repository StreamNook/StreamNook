//! The live-alert bell and Copy from Twitch: the commands every shell calls.
//! The rules are `services::live_mute`; every edit is one read-modify-write of
//! `live_notifications.muted_live_channels` under the settings lock, so no
//! shell ever sends the list from its own copy.

use crate::commands::settings::commit_settings_patch;
use crate::commands::settings_fields::patch_for_changes;
use crate::models::settings::{AppState, Settings};
use crate::rt::AppHandle;
use crate::services::live_mute::{self, CopyAnswer, CopyError, Merge};
use crate::services::twitch_service::TwitchService;
use serde_json::{json, Map, Value};
use std::time::Duration;
use tauri::State;

const LIST_PATH: &str = "live_notifications.muted_live_channels";
/// How long a copy waits for the twitch.tv web session.
const TOKEN_BUDGET: Duration = Duration::from_secs(10);

fn list_patch(current: &Settings, next: Vec<String>) -> Result<Map<String, Value>, String> {
    let mut changes = Map::new();
    changes.insert(LIST_PATH.to_string(), json!(next));
    patch_for_changes(current, &changes)
}

/// Who the alerts belong to now: the account generation, read FIRST, then
/// the signed-in Twitch account's id read with the current token (no identity
/// side effects). A refresh never moves the generation; a sign-in, a switch
/// or a sign-out does, so comparing it after a long read proves the account
/// did not change meanwhile.
async fn alert_identity() -> Result<(u64, String), CopyError> {
    use crate::services::twitch_service::TokenError;
    let generation = TwitchService::account_generation();
    // Only "no token stored" is signed out; a vault hiccup or an unreachable
    // refresh is Twitch not answering (try again).
    let token = TwitchService::get_token().await.map_err(|e| {
        match e.chain().find_map(|c| c.downcast_ref::<TokenError>()) {
            Some(TokenError::NoToken) | Some(TokenError::Refused) => CopyError::SignedOut,
            _ => CopyError::TwitchError,
        }
    })?;
    let info = TwitchService::get_user_info_with_token(&token).await.map_err(|_| CopyError::TwitchError)?;
    if info.id.is_empty() {
        return Err(CopyError::SignedOut);
    }
    Ok((generation, info.id))
}

/// Whether this channel's go-lives alert here: `true` on, `false` muted,
/// `null` when the channel has no stable identity (the shells hide the bell).
#[tauri::command]
pub async fn live_alert_state(
    provider: String,
    user_id: String,
    user_login: String,
    state: State<'_, AppState>,
) -> Result<Option<bool>, String> {
    if live_mute::canonical(&provider, &user_id, &user_login).is_none() {
        return Ok(None);
    }
    let keys = live_mute::keys_for_channel(&provider, &user_id, &user_login);
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(Some(!live_mute::is_muted(&settings.live_notifications.muted_live_channels, &keys)))
}

/// Set this channel's go-live alerts on or off and answer the stored state.
/// Idempotent: a press from a screen that is behind sets what it shows.
#[tauri::command]
pub async fn set_live_alert(
    app: AppHandle,
    provider: String,
    user_id: String,
    user_login: String,
    on: bool,
    state: State<'_, AppState>,
) -> Result<Option<bool>, String> {
    let Some(canonical) = live_mute::canonical(&provider, &user_id, &user_login) else {
        return Ok(None);
    };
    let keys = live_mute::keys_for_channel(&provider, &user_id, &user_login);
    commit_settings_patch(&app, &state, None, |current| {
        match live_mute::set_in(&current.live_notifications.muted_live_channels, &keys, Some(&canonical), on) {
            Some(next) => list_patch(current, next),
            None => Ok(Map::new()),
        }
    })?;
    live_mute::forget_copied(&keys);
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(Some(!live_mute::is_muted(&settings.live_notifications.muted_live_channels, &keys)))
}

/// Mute every followed channel whose Twitch notification bell is off. Reads
/// every page before writing anything; saves once.
#[tauri::command]
pub async fn copy_twitch_live_mutes(app: AppHandle, state: State<'_, AppState>) -> Result<CopyAnswer, CopyError> {
    let _busy = live_mute::CopyBusy::claim().ok_or(CopyError::Busy)?;
    let (generation, account) = alert_identity().await?;
    let auth = state.twitch_auth.clone();
    // Only "not signed in" asks the viewer to sign in; a slow or failed read
    // of a session that exists is Twitch not answering (try again).
    let token = match tokio::time::timeout(TOKEN_BUDGET, auth.get_token()).await {
        Ok(Ok(token)) => token,
        Ok(Err(crate::services::twitch_auth_service::AuthError::NotLoggedIn)) => return Err(CopyError::SignedOut),
        Ok(Err(e)) => {
            log::debug!("[LiveAlerts] web session unreadable for the copy: {e}");
            return Err(CopyError::TwitchError);
        }
        Err(_) => return Err(CopyError::TwitchError),
    };
    let fetched = live_mute::fetch_bells(&token).await?;
    if fetched.user_id != account {
        return Err(CopyError::WrongAccount);
    }
    let mut outcome: Result<Merge, CopyError> = Err(CopyError::SaveFailed);
    commit_settings_patch(&app, &state, None, |current| {
        // Signed in, switched or signed out while the pages were read: write nothing.
        if TwitchService::account_generation() != generation {
            outcome = Err(CopyError::WrongAccount);
            return Ok(Map::new());
        }
        let merged = live_mute::merge_bells(&current.live_notifications.muted_live_channels, &fetched.bells_off);
        let patch = if merged.added.is_empty() { Map::new() } else { list_patch(current, merged.list.clone())? };
        outcome = Ok(merged);
        Ok(patch)
    })
    .map_err(|e| {
        log::warn!("[LiveAlerts] copy save failed: {e}");
        CopyError::SaveFailed
    })?;
    let merged = outcome?;
    let answer = CopyAnswer {
        copy_id: 0,
        bells_off: fetched.bells_off.len(),
        already: merged.already,
        skipped: fetched.skipped,
        truncated: fetched.truncated,
        added: merged.added,
    };
    Ok(live_mute::record_copy(&account, generation, answer))
}

/// Take back what copy `copy_id` muted, unless another copy or an account
/// change came since. Answers how many entries it removed.
#[tauri::command]
pub async fn undo_live_mutes(app: AppHandle, copy_id: u64, state: State<'_, AppState>) -> Result<usize, CopyError> {
    let generation = TwitchService::account_generation();
    let added = live_mute::undo_list(copy_id, generation)?;
    let mut removed = 0usize;
    let mut stale = false;
    commit_settings_patch(&app, &state, None, |current| {
        // The account changed since Undo was read: take nothing back.
        if TwitchService::account_generation() != generation {
            stale = true;
            return Ok(Map::new());
        }
        let (next, count) = live_mute::undo_in(&current.live_notifications.muted_live_channels, &added);
        removed = count;
        if count == 0 {
            Ok(Map::new())
        } else {
            list_patch(current, next)
        }
    })
    .map_err(|e| {
        log::warn!("[LiveAlerts] undo save failed: {e}");
        CopyError::SaveFailed
    })?;
    if stale {
        return Err(CopyError::Stale);
    }
    live_mute::clear_copy(copy_id);
    Ok(removed)
}
