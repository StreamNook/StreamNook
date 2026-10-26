//! The vault key's slot in the OS credential store.

use super::{random_key, Key};
use base64::Engine;
use keyring_core::{CredentialStore, Error};
use log::{debug, warn};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Named like Chromium's "Chrome Safe Storage", which is what a user inspecting
/// their keychain would recognize.
const SERVICE: &str = "StreamNook Safe Storage";
const ACCOUNT: &str = "StreamNook";

/// How long a failed lookup is trusted before the store is asked again.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// A lookup that takes this long waited on a person: the macOS Keychain
/// asks whether StreamNook may read its item whenever the app's signature
/// changed (every update of an ad-hoc signed build). Every token read waits
/// behind that dialog, so it is logged.
const SLOW_ANSWER: Duration = Duration::from_secs(2);
/// A slow lookup that still failed was refused or left unanswered at that
/// dialog. Asking again every `RETRY_AFTER` would put the dialog back up
/// every half minute, each time holding every token read behind it.
const RETRY_AFTER_REFUSAL: Duration = Duration::from_secs(10 * 60);

/// Tokens that builds on keyring 2 (which had native backends on by default)
/// stored directly in the credential store, as (service, user). Nothing has
/// read them since, and their tokens may still be live.
const LEGACY_ENTRIES: &[(&str, &str)] = &[
    ("streamnook_twitch_token", "user"),
    ("streamnook_twitch_token", "pkce_verifier"),
    ("streamnook", "twitch_token"),
    ("streamnook", "twitch_refresh_token"),
    ("StreamNook", "twitch_token"),
];

/// What this platform's credential store is called, for the notice shown when
/// it does not answer.
#[cfg(windows)]
pub(super) const STORE_NAME: &str = "Windows Credential Manager";
#[cfg(target_os = "macos")]
pub(super) const STORE_NAME: &str = "the macOS Keychain";
#[cfg(not(any(windows, target_os = "macos")))]
pub(super) const STORE_NAME: &str = "a system keyring (GNOME Keyring, KWallet or KeePassXC)";

type Open = fn() -> keyring_core::Result<Arc<CredentialStore>>;

#[cfg(windows)]
fn open_platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(windows_native_keyring_store::Store::new()?)
}

#[cfg(target_os = "macos")]
fn open_platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(apple_native_keyring_store::keychain::Store::new()?)
}

#[cfg(target_os = "linux")]
fn open_platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Ok(dbus_secret_service_keyring_store::Store::new()?)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn open_platform_store() -> keyring_core::Result<Arc<CredentialStore>> {
    Err(Error::NotSupportedByStore(
        "no credential store on this platform".into(),
    ))
}

/// The store this process uses.
pub(super) fn system() -> &'static KeyStore {
    static SYSTEM: OnceLock<KeyStore> = OnceLock::new();
    SYSTEM.get_or_init(|| KeyStore::new(open_platform_store, RETRY_AFTER))
}

enum State {
    Unknown,
    Ready(Key),
    /// Failed; not asked again before this instant.
    Unavailable(Instant),
}

pub(super) struct KeyStore {
    open: Open,
    retry_after: Duration,
    /// The opened store. Opening can fail on its own (a Secret Service store
    /// connects to D-Bus up front), so it is retried like a failed lookup.
    store: Mutex<Option<Arc<CredentialStore>>>,
    state: Mutex<State>,
}

impl KeyStore {
    pub(super) fn new(open: Open, retry_after: Duration) -> Self {
        Self {
            open,
            retry_after,
            store: Mutex::new(None),
            state: Mutex::new(State::Unknown),
        }
    }

    /// The vault key. `create` makes one when the store has none; without it,
    /// a missing key is `None` and stays unknown. The state lock is held across
    /// the lookup so one process never mints two keys.
    pub(super) fn key(&self, create: bool) -> Option<Key> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let retrying = match *state {
            State::Ready(key) => return Some(key),
            State::Unavailable(retry_at) if Instant::now() < retry_at => return None,
            State::Unavailable(_) => true,
            State::Unknown => false,
        };
        let (answer, took) = self.timed_fetch(create);
        match answer {
            Ok(Some(key)) => {
                *state = State::Ready(key);
                Some(key)
            }
            Ok(None) => {
                *state = State::Unknown;
                None
            }
            Err(e) => {
                if retrying {
                    debug!("[VAULT] credential store still unavailable: {e}");
                } else {
                    warn!("[VAULT] credential store unavailable: {e}");
                }
                *state = State::Unavailable(self.retry_at(took));
                None
            }
        }
    }

    /// Whether the store answers, key or no key.
    pub(super) fn reachable(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        match *state {
            State::Ready(_) => return true,
            State::Unavailable(retry_at) if Instant::now() < retry_at => return false,
            _ => {}
        }
        let (answer, took) = self.timed_fetch(false);
        match answer {
            Ok(Some(key)) => {
                *state = State::Ready(key);
                true
            }
            Ok(None) => true,
            Err(_) => {
                *state = State::Unavailable(self.retry_at(took));
                false
            }
        }
    }

    /// Delete `LEGACY_ENTRIES`. Absence is the common case and costs one lookup.
    pub(super) fn remove_legacy_entries(&self) {
        let Ok(store) = self.store() else {
            return;
        };
        for (service, user) in LEGACY_ENTRIES {
            let Ok(entry) = store.build(service, user, None) else {
                continue;
            };
            match entry.delete_credential() {
                Ok(()) => debug!("[VAULT] removed legacy credential {service}/{user}"),
                Err(Error::NoEntry) => {}
                Err(e) => {
                    debug!("[VAULT] could not remove legacy credential {service}/{user}: {e}")
                }
            }
        }
    }

    /// `fetch`, timed: a slow answer is logged (see `SLOW_ANSWER`).
    fn timed_fetch(&self, create: bool) -> (Result<Option<Key>, String>, Duration) {
        let asked = Instant::now();
        let answer = self.fetch(create);
        let took = asked.elapsed();
        if took >= SLOW_ANSWER {
            warn!(
                "[VAULT] the credential store took {:.1}s to answer (a keychain permission prompt?); sign-ins waited for it{}",
                took.as_secs_f64(),
                if answer.is_err() { ", then it refused" } else { "" }
            );
        }
        (answer, took)
    }

    /// When a lookup that failed after `took` may be asked again.
    fn retry_at(&self, took: Duration) -> Instant {
        let wait = if took >= SLOW_ANSWER {
            self.retry_after.max(RETRY_AFTER_REFUSAL)
        } else {
            self.retry_after
        };
        Instant::now() + wait
    }

    fn store(&self) -> keyring_core::Result<Arc<CredentialStore>> {
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(open) = store.as_ref() {
            return Ok(open.clone());
        }
        let opened = (self.open)()?;
        *store = Some(opened.clone());
        Ok(opened)
    }

    /// The key is stored as base64 text, as Chromium stores its Safe Storage
    /// password, so the platform's own tools can read it (`security
    /// find-generic-password -w`, `secret-tool lookup`). An entry that does not
    /// decode is never replaced: it may be the only copy of the key that opens
    /// every sealed file, so it reads as unavailable instead.
    fn fetch(&self, create: bool) -> Result<Option<Key>, String> {
        let store = self.store().map_err(|e| e.to_string())?;
        let entry = store
            .build(SERVICE, ACCOUNT, None)
            .map_err(|e| e.to_string())?;
        match entry.get_password() {
            Ok(text) => {
                return decode_key(&text)
                    .map(Some)
                    .ok_or_else(|| "the stored vault key is malformed".to_string());
            }
            Err(Error::NoEntry) if !create => return Ok(None),
            Err(Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        }

        let key = random_key().map_err(|e| e.to_string())?;
        entry
            .set_password(&encode_key(&key))
            .map_err(|e| e.to_string())?;
        // Read back through a fresh handle: a store that accepts writes but
        // keeps nothing must never hold the only copy of the key.
        let stored = store
            .build(SERVICE, ACCOUNT, None)
            .and_then(|e| e.get_password())
            .map_err(|e| e.to_string())?;
        if decode_key(&stored) != Some(key) {
            return Err("the vault key did not read back".to_string());
        }
        debug!("[VAULT] created the vault key in the credential store");
        Ok(Some(key))
    }
}

fn encode_key(key: &Key) -> String {
    base64::engine::general_purpose::STANDARD.encode(key)
}

fn decode_key(text: &str) -> Option<Key> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()?;
    Key::try_from(bytes.as_slice()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring_core::mock;

    fn mock_store() -> keyring_core::Result<Arc<CredentialStore>> {
        Ok(mock::Store::new()?)
    }

    fn fail_to_open() -> keyring_core::Result<Arc<CredentialStore>> {
        Err(Error::NoStorageAccess("no session bus".into()))
    }

    fn slot(store: &KeyStore) -> keyring_core::Entry {
        store
            .store()
            .unwrap()
            .build(SERVICE, ACCOUNT, None)
            .unwrap()
    }

    #[test]
    fn creates_a_key_once_and_keeps_returning_it() {
        let store = KeyStore::new(mock_store, RETRY_AFTER);
        assert_eq!(store.key(false), None, "no key is made without create");
        let key = store.key(true).expect("a key is created");
        assert_eq!(store.key(false), Some(key));
        assert_eq!(decode_key(&slot(&store).get_password().unwrap()), Some(key));
    }

    #[test]
    fn an_existing_key_is_read_not_replaced() {
        let store = KeyStore::new(mock_store, RETRY_AFTER);
        let existing = [9u8; 32];
        slot(&store).set_password(&encode_key(&existing)).unwrap();
        assert_eq!(store.key(true), Some(existing));
    }

    #[test]
    fn a_malformed_entry_is_never_replaced() {
        let store = KeyStore::new(mock_store, RETRY_AFTER);
        slot(&store).set_password("not a key").unwrap();
        assert_eq!(store.key(true), None);
        assert_eq!(slot(&store).get_password().unwrap(), "not a key");
    }

    /// The mock store hands every entry for a slot the same credential, so an
    /// error set through one handle is what the next lookup sees.
    fn fail_next_lookup(store: &KeyStore) {
        slot(store)
            .as_any()
            .downcast_ref::<mock::Cred>()
            .unwrap()
            .set_error(Error::NoStorageAccess("locked".into()));
    }

    #[test]
    fn a_failed_lookup_is_retried_only_after_the_window() {
        let key = [3u8; 32];

        let patient = KeyStore::new(mock_store, Duration::from_secs(3600));
        slot(&patient).set_password(&encode_key(&key)).unwrap();
        fail_next_lookup(&patient);
        assert_eq!(patient.key(false), None, "the store failed");
        assert_eq!(
            patient.key(false),
            None,
            "inside the window it is not asked again"
        );

        let eager = KeyStore::new(mock_store, Duration::ZERO);
        slot(&eager).set_password(&encode_key(&key)).unwrap();
        fail_next_lookup(&eager);
        assert_eq!(eager.key(false), None);
        assert_eq!(
            eager.key(false),
            Some(key),
            "after the window it is asked again"
        );
    }

    /// A refusal at the keychain prompt must not bring the prompt back every
    /// half minute; a quick failure (locked keychain, no store) still retries soon.
    #[test]
    fn a_refused_prompt_waits_longer_before_asking_again() {
        let store = KeyStore::new(mock_store, RETRY_AFTER);
        let before = Instant::now();
        assert!(store.retry_at(Duration::from_millis(5)) <= before + RETRY_AFTER + Duration::from_secs(1));
        assert!(store.retry_at(SLOW_ANSWER) >= before + RETRY_AFTER_REFUSAL);
    }
    #[test]
    fn a_store_that_will_not_open_reads_as_unreachable() {
        let store = KeyStore::new(fail_to_open, RETRY_AFTER);
        assert!(!store.reachable());
        assert_eq!(store.key(true), None);
    }

    #[test]
    fn legacy_entries_are_removed() {
        let store = KeyStore::new(mock_store, RETRY_AFTER);
        let handle = store.store().unwrap();
        for (service, user) in LEGACY_ENTRIES {
            handle
                .build(service, user, None)
                .unwrap()
                .set_password("old")
                .unwrap();
        }
        store.remove_legacy_entries();
        for (service, user) in LEGACY_ENTRIES {
            let entry = handle.build(service, user, None).unwrap();
            assert!(matches!(entry.get_password(), Err(Error::NoEntry)));
        }
    }
}
