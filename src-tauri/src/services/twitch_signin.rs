//! Why the Twitch sign-in ended, when StreamNook ended it rather than the user.
//!
//! Adding a scope to `SCOPES` leaves every stored token short of a permission,
//! and the token check answers that by clearing every account (see
//! `TwitchService::verify_token_health`). Being signed out with no word of why
//! reads as a bug, so that check records the reason here.
//!
//! The reason rides the Twitch row of the account roster (`account_roster`),
//! which every window reads with `get_account_roster` and Rust pushes on
//! change, so a window opened later still knows. It is kept on disk as well:
//! closing the app before signing in must not lose it. It ends when a sign-in
//! stores a token that carries every scope.
//!
//! On disk it is sealed by the token vault like the sign-in it speaks for, so
//! it belongs to this install: a copy a backup restores onto a fresh install
//! cannot be opened there and is dropped (see `token_vault::Lost`). A first
//! launch must never say "again".

use std::fs;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::services::token_vault::CachedCredential;

/// Why StreamNook needs the user to sign in to Twitch again.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SignInReason {
    /// StreamNook now asks Twitch for a permission the stored sign-in never
    /// granted, so that sign-in was cleared.
    NewPermissions,
}

impl SignInReason {
    /// The reason spoken to the viewer as a full sentence, for a signed-out
    /// screen.
    pub fn sentence(self) -> &'static str {
        match self {
            SignInReason::NewPermissions => "Twitch needs you to sign in again for new features.",
        }
    }

    /// The reason as a short line, for a row that already says "Twitch".
    pub fn short(self) -> &'static str {
        match self {
            SignInReason::NewPermissions => "Sign in again for new features",
        }
    }
}

/// Sealed beside the token files. Read from disk until one read settles it;
/// memory is the answer after that.
static REASON: CachedCredential<SignInReason> = CachedCredential::new(".twitch_signin_owed");

/// Where builds before the vault kept the reason, as plain JSON. A backup
/// restored it onto fresh installs, so it is deleted unread.
const PLAIN_FILE_NAME: &str = ".twitch_signin_reason";

fn drop_plain_file() {
    static DROPPED: AtomicBool = AtomicBool::new(false);
    if DROPPED.load(Ordering::Acquire) {
        return;
    }
    // Before setup resolves the mobile data dir, the path is not the real one.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    if crate::services::app_paths::mobile_base().is_none() {
        return;
    }
    let Ok(dir) = crate::services::twitch_service::get_app_data_dir() else {
        return;
    };
    match fs::remove_file(dir.join(PLAIN_FILE_NAME)) {
        Ok(()) => log::info!("[Auth] Deleted the unsealed Twitch sign-in reason"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            log::warn!("[Auth] Could not delete the unsealed Twitch sign-in reason: {e}");
            return;
        }
    }
    DROPPED.store(true, Ordering::Release);
}

/// Why Twitch is waiting on a new sign-in, if StreamNook ended the last one.
pub fn current() -> Option<SignInReason> {
    drop_plain_file();
    REASON.cell().lock().map(|reason| *reason).unwrap_or(None)
}

/// Record that the user has to sign in to Twitch again, and why.
pub fn require(reason: SignInReason) {
    set(Some(reason));
}

/// A sign-in with every scope landed, so nothing is owed any more.
pub fn clear() {
    set(None);
}

fn set(next: Option<SignInReason>) {
    drop_plain_file();
    let changed = match REASON.cell().lock() {
        Ok(mut reason) => {
            let changed = *reason != next;
            *reason = next;
            changed
        }
        Err(_) => false,
    };
    if !changed {
        return;
    }
    match next {
        Some(reason) => {
            if let Err(e) = REASON.store(&reason) {
                log::warn!("[Auth] Could not save the Twitch sign-in reason: {e:#}");
            }
        }
        None => REASON.remove_file(),
    }
    crate::services::account_roster::notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reason_reaches_the_page_in_snake_case() {
        let json = serde_json::to_string(&SignInReason::NewPermissions).unwrap();
        assert_eq!(json, "\"new_permissions\"");
    }

    #[test]
    fn every_reason_has_both_wordings() {
        let reason = SignInReason::NewPermissions;
        assert_eq!(reason.sentence(), "Twitch needs you to sign in again for new features.");
        assert_eq!(reason.short(), "Sign in again for new features");
    }
}
