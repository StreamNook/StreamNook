//! Encrypted at-rest storage for every credential the app keeps on disk.
//!
//! Each credential is its own file, sealed with AES-256-GCM under one random
//! 256-bit key. The key, not the credentials, is what goes to the operating
//! system's credential store: Windows Credential Manager, the macOS Keychain, or
//! the Secret Service on Linux, as the item "StreamNook Safe Storage" (see
//! `os_store`). This is the shape Chromium and Electron's safeStorage use, and it
//! is why a store with a small size cap (a Credential Manager entry holds 2560
//! bytes) still protects the large YouTube and TikTok cookie sets.
//!
//! When no credential store answers (a Linux session without a Secret Service
//! provider, which minimal Wayland setups like Hyprland often are, or a Keychain
//! request the user denied) the key is kept in `.token_vault_key` beside the
//! credentials instead, owner-only on Unix. That stops nothing a local reader of
//! the data dir could not undo; it exists so those users stay signed in, and
//! Settings says so (`credential_storage`). Mobile always uses this file key,
//! because the app-private sandbox is the protection there.
//!
//! Every sealed file records which key sealed it. A credential store that is
//! briefly unavailable (a Secret Service that starts after the app) therefore
//! leaves its files unreadable for that run rather than lost, and a file sealed
//! under the file key moves to the credential store's key once the store
//! answers.
//!
//! The first vault access in a run migrates the whole data dir in one pass (see
//! `migrate_dir`), so no legacy or plaintext copy waits for its service to be
//! read. The formats it migrates from are in `legacy`.
//!
//! Layout: magic (4) | version (1) | key tag (1) | nonce (12) | ciphertext + tag.
//! The header and the file name are the associated data, so a sealed file cannot
//! be moved into another credential's slot.

mod legacy;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod os_store;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, Context, Result};
use log::{debug, warn};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::services::twitch_service::get_app_data_dir;

type Key = [u8; 32];

const MAGIC: [u8; 4] = [0xA7, b'S', b'N', b'V'];
const VERSION: u8 = 1;
const HEADER_LEN: usize = 6;
const NONCE_LEN: usize = 12;

/// Sealed under the key held in the OS credential store.
const TAG_OS: u8 = 1;
/// Sealed under the key in `KEY_FILE_NAME`.
const TAG_FILE: u8 = 2;

const KEY_FILE_NAME: &str = ".token_vault_key";

/// A temp file this old is left from a crashed write, not one in progress.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60);

// ----- public API -------------------------------------------------------------

/// Seal `value` as JSON into `path`, replacing whatever was there.
pub fn store_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    ensure_migrated();
    store(&SystemKeys, path, &serde_json::to_vec(value)?)
}

/// Read the credential at `path`. `Ok(None)` means there is no file. An error
/// means a file exists but could not be opened (its key is unavailable this run,
/// or it is damaged); callers treat that as signed out and must not delete it.
pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    ensure_migrated();
    load(&SystemKeys, path)?
        .map(|plain| {
            serde_json::from_slice(&plain)
                .with_context(|| format!("{} does not hold the expected JSON", path.display()))
        })
        .transpose()
}

/// Delete the credential at `path`. Absence is success.
pub fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("deleting {}", path.display())),
    }
}

/// Startup work: migrate the data dir and resolve the vault key on a background
/// thread, so the first credential read rarely waits on either (a Secret
/// Service unlock prompt, a Keychain access prompt). A read that does arrive
/// first simply waits for the same one-time pass.
pub fn warm_up() {
    std::thread::spawn(ensure_migrated);
}

/// Whether sign-ins on this device are protected by the system keyring, for
/// the notice under Accounts.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialStorage {
    /// True when no system keyring answered, so the key that encrypts sign-ins
    /// is kept in a file beside them.
    pub key_on_disk: bool,
    /// What this platform's keyring is called, for the explanation.
    pub system_store: &'static str,
}

pub fn credential_storage() -> CredentialStorage {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    return CredentialStorage {
        key_on_disk: !os_store::system().reachable(),
        system_store: os_store::STORE_NAME,
    };
    #[cfg(any(target_os = "android", target_os = "ios"))]
    return CredentialStorage {
        key_on_disk: false,
        system_store: "the app's private storage",
    };
}

/// A credential kept in memory after its file is first read, for the services
/// that consult theirs on every call (Kick, YouTube, TikTok).
///
/// A file that exists but cannot be opened yet (the credential store has not
/// answered) is read again on a later access, instead of the account reading
/// as signed out for the rest of the run. The store lookups behind those
/// retries are throttled (see `os_store`).
pub struct CachedCredential<T> {
    file_name: &'static str,
    value: Mutex<Option<T>>,
    settled: AtomicBool,
}

impl<T> CachedCredential<T> {
    pub const fn new(file_name: &'static str) -> Self {
        Self {
            file_name,
            value: Mutex::new(None),
            settled: AtomicBool::new(false),
        }
    }
}

impl<T: Serialize + DeserializeOwned> CachedCredential<T> {
    /// The in-memory credential, read from its file until one read settles it.
    pub fn cell(&self) -> &Mutex<Option<T>> {
        if !self.settled.load(Ordering::Acquire) {
            self.hydrate();
        }
        &self.value
    }

    /// Seal `value` to this credential's file. The caller updates the cell.
    pub fn store(&self, value: &T) -> Result<()> {
        store_json(&self.path()?, value)
    }

    /// Delete this credential's file. The caller clears the cell.
    pub fn remove_file(&self) {
        if let Ok(path) = self.path() {
            if let Err(e) = remove(&path) {
                warn!("[VAULT] {e:#}");
            }
        }
    }

    fn path(&self) -> Result<PathBuf> {
        Ok(get_app_data_dir()?.join(self.file_name))
    }

    fn hydrate(&self) {
        hydrate(&self.value, &self.settled, self.file_name, || {
            // Before setup resolves the mobile data dir, the path is not the real one.
            #[cfg(any(target_os = "android", target_os = "ios"))]
            if crate::services::app_paths::mobile_base().is_none() {
                return Err(anyhow!("the app data dir is not resolved yet"));
            }
            load_json::<T>(&self.path()?)
        });
    }
}

/// Fill `value` from `load` unless a read has already settled it. Only a
/// definite answer settles it: a credential, or no file. An error (the file's
/// key is unavailable right now) leaves it unsettled for the next access.
fn hydrate<T>(
    value: &Mutex<Option<T>>,
    settled: &AtomicBool,
    file_name: &str,
    load: impl FnOnce() -> Result<Option<T>>,
) {
    // try_lock: a caller already holding the cell is working on the value, and
    // waiting here would deadlock it if it came back through `cell()`.
    let Ok(mut current) = value.try_lock() else {
        return;
    };
    if settled.load(Ordering::Acquire) {
        return;
    }
    if current.is_some() {
        // Filled by a sign-in before any read settled; that is the truth.
        settled.store(true, Ordering::Release);
        return;
    }
    match load() {
        Ok(loaded) => {
            *current = loaded;
            settled.store(true, Ordering::Release);
        }
        Err(e) => debug!("[VAULT] {file_name} is not readable yet: {e:#}"),
    }
}

// ----- keys -------------------------------------------------------------------

/// Where the vault's keys come from. Production reads the OS credential store
/// and the key file; the tests substitute both.
trait Keys {
    /// The credential-store key, created when `create` and the store has none.
    /// `None` while the store does not answer.
    fn os(&self, create: bool) -> Option<Key>;
    /// The on-disk key, created when `create`.
    fn file(&self, create: bool) -> Result<Key>;
}

struct SystemKeys;

impl Keys for SystemKeys {
    fn os(&self, create: bool) -> Option<Key> {
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        return os_store::system().key(create);
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _ = create;
            None
        }
    }

    fn file(&self, create: bool) -> Result<Key> {
        file_key(create)
    }
}

fn sealing_key(keys: &dyn Keys) -> Result<(u8, Key)> {
    if let Some(key) = keys.os(true) {
        return Ok((TAG_OS, key));
    }
    Ok((TAG_FILE, keys.file(true)?))
}

fn key_for_tag(keys: &dyn Keys, tag: u8) -> Result<Key> {
    match tag {
        TAG_OS => keys
            .os(false)
            .ok_or_else(|| anyhow!("the OS credential store did not supply the vault key")),
        TAG_FILE => keys.file(false),
        other => Err(anyhow!("unknown vault key tag {other}")),
    }
}

fn random_key() -> Result<Key> {
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|e| anyhow!("no system randomness: {e}"))?;
    Ok(key)
}

static FILE_KEY: Mutex<Option<Key>> = Mutex::new(None);

/// The on-disk key. Created with `create_new`, so two processes racing to make
/// it agree on whichever landed first.
fn file_key(create: bool) -> Result<Key> {
    let mut cached = FILE_KEY.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(key) = *cached {
        return Ok(key);
    }
    let dir = get_app_data_dir()?;
    let key = read_or_create_key_file(&dir, create)?;
    *cached = Some(key);
    Ok(key)
}

fn read_or_create_key_file(dir: &Path, create: bool) -> Result<Key> {
    let path = dir.join(KEY_FILE_NAME);
    if create && !path.exists() {
        fs::create_dir_all(dir)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                let key = random_key()?;
                file.write_all(&key)?;
                file.sync_all()?;
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                warn!("[VAULT] no OS credential store answered; credentials are sealed under a key kept on disk");
                return Ok(key);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e).with_context(|| format!("creating {}", path.display())),
        }
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    Key::try_from(bytes.as_slice())
        .map_err(|_| anyhow!("{KEY_FILE_NAME} does not hold a 256-bit key"))
}

// ----- sealing ----------------------------------------------------------------

fn store(keys: &dyn Keys, path: &Path, plain: &[u8]) -> Result<()> {
    let (tag, key) = sealing_key(keys)?;
    let sealed = seal(&key, tag, file_name(path), plain)?;
    write_private(path, &sealed)
}

/// The plaintext of the credential at `path`, or `None` without a file. A
/// legacy file is decoded and sealed in place; a file sealed under the file key
/// moves to the credential store's key when that answers.
fn load(keys: &dyn Keys, path: &Path) -> Result<Option<Vec<u8>>> {
    let raw = match fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    if is_sealed(&raw) {
        let (tag, plain) = open(&raw, file_name(path), |tag| key_for_tag(keys, tag))?;
        if tag == TAG_FILE && keys.os(true).is_some() {
            match store(keys, path, &plain) {
                Ok(()) => debug!(
                    "[VAULT] moved {} to the credential store key",
                    path.display()
                ),
                Err(e) => warn!(
                    "[VAULT] could not move {} to the credential store key: {e:#}",
                    path.display()
                ),
            }
        }
        return Ok(Some(plain));
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let xor_key = legacy::xor_key(name)
        .ok_or_else(|| anyhow!("{} is not a sealed credential", path.display()))?;
    let plain = legacy::decode(&raw, xor_key);
    serde_json::from_slice::<serde::de::IgnoredAny>(&plain).with_context(|| {
        format!(
            "{} is neither sealed nor a legacy credential",
            path.display()
        )
    })?;
    match store(keys, path, &plain) {
        Ok(()) => debug!("[VAULT] sealed legacy credential {}", path.display()),
        Err(e) => warn!(
            "[VAULT] could not seal legacy credential {}: {e:#}",
            path.display()
        ),
    }
    Ok(Some(plain))
}

fn file_name(path: &Path) -> &[u8] {
    path.file_name()
        .map(|n| n.as_encoded_bytes())
        .unwrap_or_default()
}

fn is_sealed(raw: &[u8]) -> bool {
    raw.len() > HEADER_LEN + NONCE_LEN && raw[..MAGIC.len()] == MAGIC
}

fn seal(key: &Key, tag: u8, name: &[u8], plain: &[u8]) -> Result<Vec<u8>> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("no system randomness: {e}"))?;
    let header = [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], VERSION, tag];
    let aad = [&header[..], name].concat();
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| anyhow!("bad vault key length"))?;
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("sealing failed"))?;

    let mut out = Vec::with_capacity(HEADER_LEN + NONCE_LEN + sealed.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn open(raw: &[u8], name: &[u8], key_for: impl Fn(u8) -> Result<Key>) -> Result<(u8, Vec<u8>)> {
    let (header, rest) = raw.split_at(HEADER_LEN);
    if header[4] != VERSION {
        return Err(anyhow!("sealed with unknown vault version {}", header[4]));
    }
    let tag = header[5];
    let key = key_for(tag)?;
    let (nonce, sealed) = rest.split_at(NONCE_LEN);
    let aad = [header, name].concat();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow!("bad vault key length"))?;
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("sealed credential failed authentication"))?;
    Ok((tag, plain))
}

/// Write `data` to `path` through a sibling temp file and a rename, so a crash
/// mid-write never leaves a truncated credential. Owner-only on Unix.
///
/// Every write gets its own temp name. With a shared one, two writers to one
/// slot could see one truncate the other's temp file just before it is renamed
/// into place, leaving an empty credential.
fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = path.with_file_name(tmp_name);

    let written = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("writing {}", tmp.display()))?;
        file.write_all(data)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

// ----- migration --------------------------------------------------------------

static MIGRATED: AtomicBool = AtomicBool::new(false);
static MIGRATION: Mutex<()> = Mutex::new(());

/// Run the one-time pass before the first vault access of the run. Every
/// public entry point calls this; nothing it calls does, so it cannot re-enter.
fn ensure_migrated() {
    if MIGRATED.load(Ordering::Acquire) {
        return;
    }
    let _guard = MIGRATION.lock().unwrap_or_else(|p| p.into_inner());
    if MIGRATED.load(Ordering::Acquire) {
        return;
    }
    // Before setup resolves the mobile data dir, the real dir is not known.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let Some(base) = crate::services::app_paths::mobile_base() else {
        return;
    };
    let Ok(dir) = get_app_data_dir() else {
        return;
    };

    migrate_dir(&SystemKeys, &dir);

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    os_store::system().remove_legacy_entries();

    // The earlier mobile store kept a plaintext copy of each credential under
    // `secure/`, always beside a file this pass has now sealed.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let legacy_dir = base.join("secure");
        if legacy_dir.exists() {
            match fs::remove_dir_all(&legacy_dir) {
                Ok(()) => debug!("[VAULT] removed the legacy plaintext credential dir"),
                Err(e) => warn!("[VAULT] could not remove {}: {e}", legacy_dir.display()),
            }
        }
    }

    MIGRATED.store(true, Ordering::Release);
}

/// Bring every credential in `dir` to the current format: import each plaintext
/// cookie mirror into its credential when that slot is empty, then delete it;
/// seal legacy files; move files under the file key to the credential store's
/// key; delete temp files a crashed write left behind. A file that cannot be
/// opened is left exactly as it is.
fn migrate_dir(keys: &dyn Keys, dir: &Path) {
    for (jar, slot) in legacy::COOKIE_JARS {
        let jar_path = dir.join(jar);
        if !jar_path.exists() {
            continue;
        }
        let slot_path = dir.join(slot);
        if !slot_path.exists() {
            if let Some(plain) = legacy::jar_token(&jar_path) {
                if let Err(e) = store(keys, &slot_path, &plain) {
                    // Keep the mirror so the next run can try again.
                    warn!("[VAULT] could not import {jar}: {e:#}");
                    continue;
                }
                debug!("[VAULT] imported {jar} into {slot}");
            }
        }
        match fs::remove_file(&jar_path) {
            Ok(()) => debug!("[VAULT] deleted {jar}"),
            Err(e) => warn!("[VAULT] could not delete {jar}: {e}"),
        }
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if legacy::xor_key(name).is_none() {
            continue;
        }
        let path = entry.path();
        if name.ends_with(".tmp") {
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_TEMP_AGE);
            if stale {
                let _ = fs::remove_file(&path);
            }
            continue;
        }
        if let Err(e) = load(keys, &path) {
            debug!("[VAULT] left {name} as it is: {e:#}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    const KEY: Key = [7u8; 32];

    /// A credential store that answers or not on demand, and a key file held in
    /// memory.
    struct TestKeys {
        os_up: AtomicBool,
        os: Mutex<Option<Key>>,
        file: Mutex<Option<Key>>,
    }

    impl TestKeys {
        fn new(os_up: bool) -> Self {
            Self {
                os_up: AtomicBool::new(os_up),
                os: Mutex::new(None),
                file: Mutex::new(None),
            }
        }
    }

    impl Keys for TestKeys {
        fn os(&self, create: bool) -> Option<Key> {
            if !self.os_up.load(Ordering::SeqCst) {
                return None;
            }
            let mut key = self.os.lock().unwrap();
            if key.is_none() && create {
                *key = Some(random_key().unwrap());
            }
            *key
        }

        fn file(&self, create: bool) -> Result<Key> {
            let mut key = self.file.lock().unwrap();
            if key.is_none() && create {
                *key = Some(random_key().unwrap());
            }
            key.ok_or_else(|| anyhow!("no key file"))
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("sn_vault_{}_{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn legacy_file(dir: &Path, name: &str, json: &str) -> PathBuf {
        let path = dir.join(name);
        let key = legacy::xor_key(name).unwrap();
        fs::write(&path, legacy::decode(json.as_bytes(), key)).unwrap();
        path
    }

    fn tag_of(path: &Path) -> u8 {
        let raw = fs::read(path).unwrap();
        assert!(is_sealed(&raw), "{} is not sealed", path.display());
        raw[5]
    }

    fn fixed(tag: u8) -> Result<Key> {
        match tag {
            TAG_OS => Ok(KEY),
            _ => Err(anyhow!("no key for tag {tag}")),
        }
    }

    // ----- format

    #[test]
    fn seal_round_trips() {
        let sealed = seal(&KEY, TAG_OS, b".kick_token", b"{\"a\":1}").unwrap();
        assert!(is_sealed(&sealed));
        let (tag, plain) = open(&sealed, b".kick_token", fixed).unwrap();
        assert_eq!(tag, TAG_OS);
        assert_eq!(plain, b"{\"a\":1}");
    }

    #[test]
    fn sealing_is_randomized() {
        let a = seal(&KEY, TAG_OS, b"n", b"same").unwrap();
        let b = seal(&KEY, TAG_OS, b"n", b"same").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_file_moved_to_another_slot_does_not_open() {
        let sealed = seal(&KEY, TAG_OS, b".twitch_account_1", b"{}").unwrap();
        assert!(open(&sealed, b".twitch_account_2", fixed).is_err());
    }

    #[test]
    fn a_tampered_header_does_not_open() {
        let mut sealed = seal(&KEY, TAG_OS, b"n", b"{}").unwrap();
        sealed[5] = TAG_FILE;
        assert!(open(&sealed, b"n", |_| Ok(KEY)).is_err());
    }

    #[test]
    fn a_wrong_key_does_not_open() {
        let sealed = seal(&KEY, TAG_OS, b"n", b"{}").unwrap();
        assert!(open(&sealed, b"n", |_| Ok([8u8; 32])).is_err());
    }

    #[test]
    fn legacy_files_are_not_mistaken_for_sealed_ones() {
        // Every legacy file is JSON XORed under an ASCII key, so its first byte
        // is '{' ^ an ASCII letter, never the non-ASCII magic byte.
        for (name, _) in [
            (".twitch_token", ()),
            (".twitch_account_1", ()),
            (".twitch_drops_token", ()),
            (".seventv_token", ()),
            (".seventv_token_1", ()),
            (".modroom_token", ()),
            (".kick_token", ()),
            (".youtube_session", ()),
            (".tiktok_session", ()),
        ] {
            let key = legacy::xor_key(name).unwrap();
            let json = b"{\"access_token\":\"abcdefghijklmnopqrstu\"}";
            let raw = legacy::decode(json, key);
            assert!(!is_sealed(&raw), "{name}");
            assert_eq!(legacy::decode(&raw, key), json);
        }
    }

    #[test]
    fn concurrent_writes_to_one_slot_never_leave_a_partial_file() {
        let dir = TempDir::new("concurrent");
        let path = dir.0.join(".seventv_token");
        let payloads: Vec<Vec<u8>> = (0..16u8).map(|i| vec![i; 4096]).collect();

        std::thread::scope(|scope| {
            for payload in &payloads {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..20 {
                        let _ = write_private(path, payload);
                    }
                });
            }
        });

        let written = fs::read(&path).unwrap();
        assert!(payloads.contains(&written), "the slot holds a torn write");
        let leftovers = fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "temp files were left behind");
    }

    // ----- keys and fallback

    #[test]
    fn writes_use_the_os_key_when_the_store_answers() {
        let dir = TempDir::new("os_key");
        let keys = TestKeys::new(true);
        let path = dir.0.join(".kick_token");
        store(&keys, &path, b"{\"a\":1}").unwrap();
        assert_eq!(tag_of(&path), TAG_OS);
        assert_eq!(load(&keys, &path).unwrap().unwrap(), b"{\"a\":1}");
    }

    #[test]
    fn writes_fall_back_to_the_file_key_when_the_store_is_down() {
        let dir = TempDir::new("fallback");
        let keys = TestKeys::new(false);
        let path = dir.0.join(".kick_token");
        store(&keys, &path, b"{\"a\":1}").unwrap();
        assert_eq!(tag_of(&path), TAG_FILE);
        assert_eq!(load(&keys, &path).unwrap().unwrap(), b"{\"a\":1}");
    }

    #[test]
    fn a_file_key_file_moves_to_the_os_key_once_the_store_answers() {
        let dir = TempDir::new("upgrade");
        let keys = TestKeys::new(false);
        let path = dir.0.join(".youtube_session");
        store(&keys, &path, b"{\"s\":2}").unwrap();

        keys.os_up.store(true, Ordering::SeqCst);
        migrate_dir(&keys, &dir.0);
        assert_eq!(tag_of(&path), TAG_OS);
        assert_eq!(load(&keys, &path).unwrap().unwrap(), b"{\"s\":2}");
    }

    #[test]
    fn a_file_whose_key_is_unavailable_is_left_untouched() {
        let dir = TempDir::new("unavailable");
        let keys = TestKeys::new(true);
        let path = dir.0.join(".twitch_token");
        store(&keys, &path, b"{\"t\":3}").unwrap();
        let before = fs::read(&path).unwrap();

        keys.os_up.store(false, Ordering::SeqCst);
        assert!(
            load(&keys, &path).is_err(),
            "reads as unavailable, not as empty"
        );
        migrate_dir(&keys, &dir.0);
        assert_eq!(fs::read(&path).unwrap(), before, "not rewritten or deleted");

        keys.os_up.store(true, Ordering::SeqCst);
        assert_eq!(load(&keys, &path).unwrap().unwrap(), b"{\"t\":3}");
    }

    // ----- cached credentials

    #[test]
    fn an_unreadable_credential_is_read_again_on_the_next_access() {
        let value = Mutex::new(None::<u32>);
        let settled = AtomicBool::new(false);

        hydrate(&value, &settled, "t", || {
            Err(anyhow!("store not answering"))
        });
        assert_eq!(*value.lock().unwrap(), None);
        assert!(
            !settled.load(Ordering::SeqCst),
            "a failure does not settle it"
        );

        hydrate(&value, &settled, "t", || Ok(Some(5)));
        assert_eq!(*value.lock().unwrap(), Some(5));
        assert!(settled.load(Ordering::SeqCst));

        hydrate(&value, &settled, "t", || {
            panic!("a settled cell is not read again")
        });
    }

    #[test]
    fn no_file_settles_as_signed_out() {
        let value = Mutex::new(None::<u32>);
        let settled = AtomicBool::new(false);
        hydrate(&value, &settled, "t", || Ok(None));
        assert!(settled.load(Ordering::SeqCst));
        assert_eq!(*value.lock().unwrap(), None);
    }

    #[test]
    fn a_sign_in_before_the_first_read_wins() {
        let value = Mutex::new(Some(9u32));
        let settled = AtomicBool::new(false);
        hydrate(&value, &settled, "t", || {
            panic!("the signed-in value is not overwritten")
        });
        assert!(settled.load(Ordering::SeqCst));
        assert_eq!(*value.lock().unwrap(), Some(9));
    }

    #[test]
    fn a_held_cell_is_not_waited_on() {
        let value = Mutex::new(None::<u32>);
        let settled = AtomicBool::new(false);
        let _held = value.lock().unwrap();
        hydrate(&value, &settled, "t", || {
            panic!("must not read while the cell is held")
        });
        assert!(!settled.load(Ordering::SeqCst));
    }

    // ----- migration

    #[test]
    fn migration_seals_every_legacy_file() {
        let dir = TempDir::new("legacy");
        let keys = TestKeys::new(true);
        let names = [
            ".twitch_token",
            ".twitch_account_42",
            ".twitch_drops_token",
            ".seventv_token",
            ".seventv_token_42",
            ".modroom_token",
            ".kick_token",
            ".youtube_session",
            ".tiktok_session",
        ];
        for name in names {
            legacy_file(&dir.0, name, &format!("{{\"file\":\"{name}\"}}"));
        }
        migrate_dir(&keys, &dir.0);
        for name in names {
            let path = dir.0.join(name);
            assert_eq!(tag_of(&path), TAG_OS, "{name}");
            let plain = load(&keys, &path).unwrap().unwrap();
            assert_eq!(
                plain,
                format!("{{\"file\":\"{name}\"}}").as_bytes(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_a_legacy_credential_is_left_alone() {
        let dir = TempDir::new("garbage");
        let keys = TestKeys::new(true);
        let path = dir.0.join(".kick_token");
        fs::write(&path, b"\x00\x01not json at all").unwrap();
        let other = dir.0.join("settings.json");
        fs::write(&other, b"{\"plain\":true}").unwrap();

        migrate_dir(&keys, &dir.0);
        assert_eq!(fs::read(&path).unwrap(), b"\x00\x01not json at all");
        assert_eq!(fs::read(&other).unwrap(), b"{\"plain\":true}");
    }

    /// A mirror as it could exist on disk: `save_json` keeps only persistent
    /// cookies, so each one carries a lifetime.
    fn write_jar(path: &Path, access: &str) {
        let mut store = cookie_store::CookieStore::default();
        let url = url::Url::parse("https://twitch.tv").unwrap();
        for (name, value) in [
            ("auth-token", access),
            ("refresh-token", "r1"),
            ("token-expires-at", "1700000000"),
        ] {
            let cookie = cookie_store::RawCookie::parse(format!(
                "{name}={value}; Domain=twitch.tv; Path=/; Max-Age=2592000"
            ))
            .unwrap();
            store.insert_raw(&cookie, &url).unwrap();
        }
        let mut file = fs::File::create(path).unwrap();
        store.save_json(&mut file).unwrap();
    }

    #[test]
    fn a_cookie_mirror_fills_an_empty_slot_and_is_deleted() {
        let dir = TempDir::new("jar_import");
        let keys = TestKeys::new(true);
        write_jar(&dir.0.join("cookies.json"), "a1");

        migrate_dir(&keys, &dir.0);
        assert!(!dir.0.join("cookies.json").exists());
        let plain = load(&keys, &dir.0.join(".twitch_token")).unwrap().unwrap();
        let token: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(token["access_token"], "a1");
        assert_eq!(token["refresh_token"], "r1");
        assert_eq!(token["expires_at"], 1_700_000_000);
    }

    #[test]
    fn a_cookie_mirror_never_overwrites_a_slot_that_holds_a_token() {
        let dir = TempDir::new("jar_stale");
        let keys = TestKeys::new(true);
        let slot = dir.0.join(".twitch_drops_token");
        store(&keys, &slot, b"{\"access_token\":\"current\"}").unwrap();
        write_jar(&dir.0.join("cookies_drops.json"), "stale");

        migrate_dir(&keys, &dir.0);
        assert!(!dir.0.join("cookies_drops.json").exists());
        assert_eq!(
            load(&keys, &slot).unwrap().unwrap(),
            b"{\"access_token\":\"current\"}"
        );
    }

    #[test]
    fn only_stale_temp_files_are_removed() {
        let dir = TempDir::new("temps");
        let keys = TestKeys::new(true);
        let stale = dir.0.join(".twitch_token.999.0.tmp");
        let fresh = dir.0.join(".twitch_token.999.1.tmp");
        fs::write(&stale, b"x").unwrap();
        fs::write(&fresh, b"x").unwrap();
        fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(3600))
            .unwrap();

        migrate_dir(&keys, &dir.0);
        assert!(!stale.exists());
        assert!(fresh.exists(), "a write may still be in progress");
    }
}
