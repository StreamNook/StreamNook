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

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

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

/// Plain JSON beside the token files. Not a secret: it names a reason, never a
/// credential.
const FILE_NAME: &str = ".twitch_signin_reason";

fn file_path() -> Option<PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|dir| dir.join(FILE_NAME))
}

/// A missing or unreadable file means no reason is owed.
fn read_from(path: &Path) -> Option<SignInReason> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim()).ok()
}

fn write_to(path: &Path, reason: Option<SignInReason>) -> io::Result<()> {
    match reason {
        Some(reason) => {
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            let json = serde_json::to_string(&reason).map_err(io::Error::other)?;
            fs::write(path, json)
        }
        None => match fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    }
}

/// Read from disk once, on first use; memory is the answer after that.
fn cell() -> &'static Mutex<Option<SignInReason>> {
    static CELL: OnceLock<Mutex<Option<SignInReason>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(file_path().and_then(|p| read_from(&p))))
}

/// Why Twitch is waiting on a new sign-in, if StreamNook ended the last one.
pub fn current() -> Option<SignInReason> {
    cell().lock().map(|reason| *reason).unwrap_or(None)
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
    let changed = match cell().lock() {
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
    if let Some(path) = file_path() {
        if let Err(e) = write_to(&path, next) {
            log::warn!("[Auth] Could not save the Twitch sign-in reason: {e}");
        }
    }
    crate::services::account_roster::notify();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_file(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir()
            .join(format!("sn-signin-{}-{nanos}-{name}", std::process::id()))
            .join(FILE_NAME)
    }

    #[test]
    fn reason_round_trips_through_its_file() {
        let path = scratch_file("roundtrip");
        assert_eq!(read_from(&path), None);

        write_to(&path, Some(SignInReason::NewPermissions)).unwrap();
        assert_eq!(read_from(&path), Some(SignInReason::NewPermissions));

        write_to(&path, None).unwrap();
        assert!(!path.exists());
        assert_eq!(read_from(&path), None);
        // Clearing what is already clear is not an error.
        write_to(&path, None).unwrap();

        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_damaged_file_owes_nothing() {
        let path = scratch_file("damaged");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "not json").unwrap();
        assert_eq!(read_from(&path), None);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

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
