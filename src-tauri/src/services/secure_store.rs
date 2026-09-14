//! Secure token storage with a platform-split backend.
//!
//! Desktop uses the OS keyring (Windows Credential Manager / macOS Keychain /
//! Linux Secret Service) via the `keyring` crate. `keyring` has no
//! production-grade Android backend, so on mobile this falls back to one file
//! per credential under the app-private data dir (already sandboxed per-app on
//! Android, so no other app can read it without root).
//!
//! Every caller uses the forgiving `if let Ok(entry) = Entry::new(..)` /
//! `if let Ok(v) = entry.get_password()` pattern and treats any error as "no
//! credential", so the backends only need matching method shapes — not
//! `keyring`'s exact error enum.

// Desktop: re-export the real keyring Entry so behavior is byte-for-byte the
// same as before this abstraction existed.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use keyring::Entry;

#[cfg(any(target_os = "android", target_os = "ios"))]
pub use mobile::Entry;

#[cfg(any(target_os = "android", target_os = "ios"))]
mod mobile {
    use std::io;
    use std::path::PathBuf;

    /// File-backed stand-in for the subset of `keyring::Entry` the app uses.
    /// One file per (service, user) under `<app_data>/secure/`.
    pub struct Entry {
        path: PathBuf,
    }

    fn sanitize(s: &str) -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    }

    impl Entry {
        pub fn new(service: &str, user: &str) -> Result<Self, io::Error> {
            let base = crate::services::app_paths::mobile_base()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "app data dir not set"))?
                .join("secure");
            std::fs::create_dir_all(&base)?;
            let file = format!("{}__{}.tok", sanitize(service), sanitize(user));
            Ok(Self {
                path: base.join(file),
            })
        }

        pub fn set_password(&self, value: &str) -> Result<(), io::Error> {
            std::fs::write(&self.path, value.as_bytes())
        }

        pub fn get_password(&self) -> Result<String, io::Error> {
            std::fs::read_to_string(&self.path)
        }

        /// Mirrors `keyring::Entry::delete_credential`. Absence is success —
        /// callers only ever want the credential gone.
        pub fn delete_credential(&self) -> Result<(), io::Error> {
            match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            }
        }
    }
}
