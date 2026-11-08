//! Every whisper command the apps call. The work lives in the services
//! (whisper_inbox, whisper_send, whisper_sync, whisper_claims); these only
//! hand it to the page or the shell. The account is Rust's own followed
//! Twitch account, never one the page passes.

use crate::rt::Window;
use crate::services::whisper_inbox::{self, WhisperList, WhisperPage, WhisperUpdate};
use crate::services::whisper_send::{self, SendTarget};
use crate::services::whisper_sync::{self, SyncOutcome};

/// Default and largest page of messages one `whisper_page` call returns.
const PAGE_DEFAULT: u32 = 100;
const PAGE_MAX: u32 = 200;

/// The followed Twitch account, or empty when nobody is signed in.
async fn owner() -> String {
    if let Some(scope) = whisper_sync::current_scope() {
        return scope.owner;
    }
    match crate::services::account_roster::twitch_user_id() {
        Some(id) => id,
        None => crate::services::twitch_service::TwitchService::get_user_info()
            .await
            .map(|u| u.id)
            .unwrap_or_default(),
    }
}

/// The signed-in account's conversations, newest first, with the unread total.
#[tauri::command]
pub async fn whisper_list() -> Result<WhisperList, String> {
    let mut list = whisper_inbox::list()?;
    // The Following list's live answer for these people, no network.
    crate::services::whisper_live::fill(&mut list.conversations).await;
    Ok(list)
}

/// One page of a conversation's messages, oldest first, older than `cursor`.
#[tauri::command]
pub async fn whisper_page(key: String, cursor: Option<String>, limit: Option<u32>) -> Result<WhisperPage, String> {
    let limit = limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX) as usize;
    whisper_inbox::page(&key, cursor, limit)
}

/// Which conversation this window shows (None when it shows none). `page_load`
/// and `seq` order the calls of one page, so a late call never undoes a newer one.
#[tauri::command]
pub async fn whisper_set_active(window: Window, key: Option<String>, page_load: String, seq: u64) -> Result<(), String> {
    let owner = owner().await;
    crate::services::whisper_claims::set_claim(window.label(), &owner, key, &page_load, seq);
    Ok(())
}

/// Mark one conversation read, here and on Twitch.
#[tauri::command]
pub async fn whisper_mark_read(key: String) -> Result<(), String> {
    whisper_inbox::mark_read(&key)
}

/// Mark every conversation read, here and on Twitch.
#[tauri::command]
pub async fn whisper_mark_all_read() -> Result<(), String> {
    whisper_inbox::mark_all_read()
}

/// Send a whisper. The message is filed as Sending at once; this returns once
/// Twitch has answered (Sent, or the reason it refused). A notification reply
/// passes the account the whisper came to, so a reply after an account switch
/// is refused.
#[tauri::command]
pub async fn whisper_send(
    target: SendTarget,
    text: String,
    expect_owner: Option<String>,
    expect_owner_name: Option<String>,
) -> Result<WhisperUpdate, String> {
    whisper_send::send(target, text, expect_owner, expect_owner_name).await
}

/// An inbox is on screen (the island's history, MultiChat's Mentions mode,
/// a phone's inbox, the Whispers window): sync if a minute has passed.
#[tauri::command]
pub async fn inbox_opened() -> Result<(), String> {
    whisper_sync::trigger(whisper_sync::Reason::InboxOpened);
    Ok(())
}

/// Send a Not delivered whisper again (after checking Twitch does not have it).
#[tauri::command]
pub async fn whisper_retry(key: String, message_id: String) -> Result<WhisperUpdate, String> {
    whisper_send::retry(key, message_id).await
}

/// Drop a Not delivered whisper.
#[tauri::command]
pub async fn whisper_discard(key: String, message_id: String) -> Result<(), String> {
    whisper_send::discard(&key, &message_id)
}

/// Archive a conversation on Twitch and here.
#[tauri::command]
pub async fn whisper_archive(key: String) -> Result<(), String> {
    whisper_inbox::archive(&key).await
}

/// Load one more page of a conversation's older history from Twitch.
#[tauri::command]
pub async fn whisper_load_older(key: String) -> Result<u32, String> {
    whisper_sync::load_older(key).await
}

/// Sync the signed-in account's whispers with Twitch now.
#[tauri::command]
pub async fn whisper_sync_now() -> Result<SyncOutcome, String> {
    Ok(whisper_sync::sync_now().await)
}

/// The island's Clear all: hide the whisper rows it shows now from the island.
#[tauri::command]
pub async fn whisper_island_clear() -> Result<(), String> {
    whisper_inbox::island_clear()
}

/// The phone app can be read now: in front and not in picture-in-picture.
/// Allowed only by mobile-commands.toml.
#[cfg(mobile)]
#[tauri::command]
pub async fn set_app_interactive(interactive: bool) -> Result<(), String> {
    crate::services::whisper_claims::set_app_interactive(interactive);
    Ok(())
}

/// The phone shell's answer to a Twitch security-check request: the page's
/// final address, or why it failed. False when no request is waiting on it.
/// Allowed only by mobile-commands.toml.
#[cfg(mobile)]
#[tauri::command]
pub async fn twitch_integrity_minted(request_id: String, url: Option<String>, error: Option<String>) -> Result<bool, String> {
    let outcome = match url {
        Some(u) => Ok(u),
        None => Err(error.unwrap_or_else(|| "no answer".into())),
    };
    Ok(crate::services::twitch_integrity::matcher::answer(&request_id, outcome))
}
