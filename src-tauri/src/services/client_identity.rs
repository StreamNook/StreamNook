//! What this client IS: version, OS, architecture, build channel.
//!
//! One definition, because there used to be three and they disagreed. The
//! admin dashboard was reading whichever won a race:
//!
//! - `users.app_version` came from JS `getVersion()` (the merged Tauri config).
//! - The presence payload came from `get_current_app_version`
//!   (`env!("CARGO_PKG_VERSION")`).
//! - The updater kept a private third copy of `CARGO_PKG_VERSION`.
//!
//! On desktop those agree only because the release scripts happen to bump both
//! files; `tests/version_parity.rs` now makes that a hard guarantee instead of
//! a habit. On ANDROID they never agreed: `tauri.android.conf.json` overrides
//! the version for Gradle and never reaches Cargo, so the Cargo number is the
//! DESKTOP one inside a phone build. Android reported 8.3.9 to Supabase once
//! for exactly this reason.
//!
//! So `app_version` here reads the MERGED CONFIG (`package_info().version`),
//! which is correct on both platforms from one line of code, and every reporter
//! reads it from here.
//!
//! `target` is the `<os>-<arch>` key the update manifest already uses, produced
//! by the same function the updater calls (`current_update_target`, moved here
//! from `commands::components`). Update artifacts and the report therefore share
//! one key space by construction, which is what makes "which platform is stuck
//! on an old version" answerable at all: before this, every desktop OS reported
//! the single string `"desktop"`, so Windows, macOS and Linux were
//! indistinguishable both online and offline.

use serde::Serialize;
use tauri::{AppHandle, Manager};

/// The `<os>-<arch>` key for the running platform.
///
/// Same construction as `plugin_host::install::IndexEntry::artifact_for_platform`
/// and the update manifest's `platforms` map, deliberately: one key space.
/// `std::env::consts` gives Rust's own names (`windows`, `macos`, `linux`,
/// `android`, `ios`; `x86_64`, `aarch64`), which is what the release pipeline
/// writes into `latest.json`.
pub fn current_update_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Now, as the RFC3339 string every timestamp we report uses.
///
/// Here rather than at each call site so the reported timestamps cannot end up in
/// two formats. `SecondsFormat::Secs` because these land in a `timestamptz` and
/// sub-second precision on a 15-minute heartbeat is noise.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Debug builds must be distinguishable in the fleet data. Brandon's own dev
/// build otherwise looks like a user on an unreleased version, and a sideloaded
/// debug APK looks like a phantom release.
fn build_channel() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Everything a reporter needs to say which client this is. Serialized straight
/// to the frontend and straight into the report payload, so the field names
/// are the wire format: renaming one is a cross-repo change.
#[derive(Debug, Clone, Serialize)]
pub struct ClientIdentity {
    /// The merged-config version: 8.x on desktop, 0.1.x on Android.
    pub app_version: String,
    /// `windows` | `macos` | `linux` | `android` | `ios`, from Rust's own
    /// target names. NOT a user-agent sniff: the two UA sniffs this replaces
    /// disagreed with each other on iOS, one of them labelling an iPhone
    /// `android`.
    pub platform: String,
    /// `x86_64` | `aarch64`. A macOS user on Rosetta versus native is the
    /// difference between the two macOS update artifacts.
    pub arch: String,
    /// `<platform>-<arch>`, the update manifest's own key.
    pub target: String,
    /// `release` | `debug`.
    pub channel: String,
}

/// Read the running client's identity.
///
/// Cheap and allocation-only: `package_info` is baked in at compile time by
/// `tauri::generate_context!` and `std::env::consts` are constants, so there is
/// nothing to cache and no failure mode.
pub fn current(app: &AppHandle) -> ClientIdentity {
    let platform = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    ClientIdentity {
        app_version: app.package_info().version.to_string(),
        target: current_update_target(),
        platform,
        arch,
        channel: build_channel().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_os_dash_arch() {
        let target = current_update_target();
        let (os, arch) = target
            .split_once('-')
            .unwrap_or_else(|| panic!("target must be `<os>-<arch>`, got {target}"));
        assert_eq!(os, std::env::consts::OS);
        assert_eq!(arch, std::env::consts::ARCH);
    }

    #[test]
    fn channel_reflects_the_build() {
        // Asserted against the same cfg the function reads, so this fails if
        // the two ever stop agreeing rather than asserting a constant.
        if cfg!(debug_assertions) {
            assert_eq!(build_channel(), "debug");
        } else {
            assert_eq!(build_channel(), "release");
        }
    }
}
