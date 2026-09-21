use crate::models::components::{
    BundleUpdateStatus, ComponentChanges, ComponentManifest, VersionChange,
};
use sevenz_rust::decompress_file;
use std::path::{Path, PathBuf};

/// Get the directory where the executable is located (portable mode)
fn get_exe_directory() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Failed to get exe directory".to_string())
}

/// Get the path to the local components.json (next to exe in portable mode)
fn get_components_json_path() -> Result<PathBuf, String> {
    let exe_dir = get_exe_directory()?;
    Ok(exe_dir.join("components.json"))
}

/// Whether onboarding's "components" step is satisfied. StreamNook is now a
/// self-contained native client (no external Streamlink/plugin to provision), so
/// there is nothing to install — always true.
#[tauri::command]
pub fn check_components_installed() -> Result<bool, String> {
    Ok(true)
}

/// Get local component versions from components.json
#[tauri::command]
pub fn get_local_component_versions() -> Result<ComponentManifest, String> {
    let components_path = get_components_json_path()?;

    if !components_path.exists() {
        return Err("Components not installed".to_string());
    }

    ComponentManifest::load_from_file(&components_path)
        .map_err(|e| format!("Failed to load components.json: {}", e))
}

/// Fetch remote component versions from GitHub
#[tauri::command]
pub async fn get_remote_component_versions() -> Result<ComponentManifest, String> {
    let mut builder = reqwest::Client::builder().user_agent("StreamNook");

    // Inject PAT to bypass 60-req/hour limit during intense development
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        builder = builder.default_headers(
            std::iter::once((
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            ))
            .collect(),
        );
    }

    let client = builder.build().map_err(|e| e.to_string())?;

    // Directly download components.json from the latest release asset redirect
    // This entirely bypasses the api.github.com rate limit for unauthenticated users
    let components_json: ComponentManifest = client
        .get("https://github.com/winters27/StreamNook/releases/latest/download/components.json")
        .send()
        .await
        .map_err(|e| format!("Failed to download components.json: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse components.json: {}", e))?;

    Ok(components_json)
}

/// Try to copy components.json from exe directory to AppData if missing
fn try_copy_components_from_exe() -> Option<ComponentManifest> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    let source_components = exe_dir.join("components.json");

    if source_components.exists() {
        // Try to load from exe directory
        if let Ok(manifest) = ComponentManifest::load_from_file(&source_components) {
            // Try to copy to AppData for future use
            if let Ok(dest_path) = get_components_json_path() {
                if let Some(parent) = dest_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&source_components, &dest_path);
            }
            return Some(manifest);
        }
    }
    None
}

/// Get the current app version from Cargo.toml
fn get_current_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Parse a dotted version ("8.0.1") into comparable numeric components, dropping
/// any leading `v` and any pre-release/build suffix. Returns None if it can't be
/// read as numeric dotted parts.
fn parse_version(v: &str) -> Option<Vec<u64>> {
    let core = v.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next().unwrap_or(core);
    let parts = core
        .split('.')
        .map(|p| p.parse::<u64>().ok())
        .collect::<Option<Vec<u64>>>()?;
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

/// True only when `remote` is a strictly newer release than `current`, compared
/// as semantic versions. This is what stops a stale or rolled-back manifest from
/// prompting a downgrade: an equal or lower remote version offers no update. If
/// either side can't be parsed numerically, fall back to string inequality so an
/// unusual version string still surfaces an update rather than hiding one.
fn remote_is_newer(remote: &str, current: &str) -> bool {
    match (parse_version(remote), parse_version(current)) {
        (Some(r), Some(c)) => {
            let n = r.len().max(c.len());
            for i in 0..n {
                let rv = r.get(i).copied().unwrap_or(0);
                let cv = c.get(i).copied().unwrap_or(0);
                if rv != cv {
                    return rv > cv;
                }
            }
            false
        }
        _ => remote != current,
    }
}

/// The update manifest served from streamnook.app/api/v1/update, generated by
/// the release pipeline and stored in R2. This is the primary update source:
/// asking our own domain instead of a GitHub release means the repo can move,
/// be renamed, or be archived without breaking any installed client.
const UPDATE_MANIFEST_URL: &str = "https://streamnook.app/api/v1/update";

/// One downloadable build. Everything here is per-platform; only `version` and
/// `notes` are shared across them.
#[derive(Clone, Debug, Default, serde::Deserialize)]
struct UpdateArtifact {
    download_url: String,
    #[serde(default)]
    bundle_name: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    /// Detached minisign signature over the bundle. See BundleUpdateStatus.
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(serde::Deserialize)]
struct UpdateManifest {
    version: String,
    /// Per-platform builds, keyed `<os>-<arch>` (`windows-x86_64`,
    /// `macos-aarch64`, `macos-x86_64`, `linux-x86_64`).
    ///
    /// Deliberately the SAME key space the plugin index uses
    /// (`plugin_host::install::IndexEntry::platforms`). One scheme used twice
    /// beats two schemes that mean the same thing.
    #[serde(default)]
    platforms: std::collections::HashMap<String, UpdateArtifact>,

    // ── Legacy flat fields ──────────────────────────────────────────────────
    //
    // These describe a Windows x86_64 bundle and MUST stay. Every client
    // already installed in the wild reads them and knows nothing about
    // `platforms`; removing them would strand that entire population on their
    // current build with no way to update. The pipeline keeps writing them for
    // Windows, and `artifact_for_platform` treats them as the windows-x86_64
    // entry when the map has none.
    download_url: String,
    #[serde(default)]
    bundle_name: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    signature: Option<String>,
    #[serde(default)]
    size: Option<u64>,

    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    min_supported: Option<String>,
}

/// The `<os>-<arch>` key for the running platform.
///
/// Same construction as `plugin_host::install::IndexEntry::artifact_for_platform`,
/// deliberately: the update manifest and the plugin index share a key space.
fn current_update_target() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Default artifact filename when the manifest does not name one.
fn default_bundle_name() -> &'static str {
    if cfg!(target_os = "macos") {
        // A .app is a directory, so it travels as a tarball. NOT a .dmg: a dmg
        // is a delivery format for a human clicking a download link, and asking
        // an updater to mount a disk image to extract one bundle is strictly
        // more that can fail.
        "StreamNook.app.tar.gz"
    } else if cfg!(target_os = "linux") {
        "StreamNook.AppImage"
    } else {
        "StreamNook.7z"
    }
}

impl UpdateManifest {
    /// The build for the platform we are running on, or `None` if the manifest
    /// does not publish one.
    ///
    /// `None` means "no update offered", which is the correct and SAFE answer.
    /// The alternative — falling back to the flat fields on any platform —
    /// would hand a macOS client `StreamNook.7z` and have it try to install a
    /// Windows build over itself. The legacy fallback is therefore gated to
    /// `windows-x86_64` and nothing else.
    fn artifact_for_platform(&self) -> Option<UpdateArtifact> {
        self.artifact_for_target(&current_update_target())
    }

    /// Resolution logic, taking the target explicitly.
    ///
    /// Split out from `artifact_for_platform` so it can be tested for EVERY
    /// platform from any one of them. Reading `std::env::consts` inside would
    /// mean the macOS safety rule below could only ever be tested on macOS,
    /// which is precisely the platform where nobody was running the tests.
    fn artifact_for_target(&self, target: &str) -> Option<UpdateArtifact> {
        if let Some(found) = self.platforms.get(target) {
            return Some(found.clone());
        }
        (target == "windows-x86_64").then(|| UpdateArtifact {
            download_url: self.download_url.clone(),
            bundle_name: self.bundle_name.clone(),
            sha256: self.sha256.clone(),
            signature: self.signature.clone(),
            size: self.size,
        })
    }
}

// ── Update artifact verification ────────────────────────────────────────────
//
// The updater downloads a 7z, swaps StreamNook.exe, and restarts. Whatever can
// answer for the update URL therefore gets code execution on every install, so
// this is the highest-consequence check in the app.
//
// A hash alone does not provide that. `sha256` and the bundle both come from the
// same host, so whoever can serve a malicious bundle can serve a matching hash;
// it detects corruption, not tampering. A minisign signature does, because the
// secret key exists only in CI and is never on the update host.

/// Pinned minisign public key for update bundles, in the same spirit as the
/// plugin operator key in `plugin_host::install::OFFICIAL_INDEX`: compiled in,
/// never fetched, so trust does not depend on the server being honest.
///
/// Key `3C746A225DA1775E`, generated 2026-08-30. Its secret half exists only as
/// the `UPDATE_SIGNING_KEY` CI secret and in Brandon's own backup; it has never
/// been committed and must never reach the host serving updates.
///
/// Rotating means shipping a new value here, so every installed client keeps
/// trusting the old key until it updates. That is survivable only while
/// `ENFORCE_UPDATE_SIGNATURE` is false, which is the other reason not to flip
/// that until signing has been proven across a release or two.
const UPDATE_PUBKEY: Option<&str> =
    Some("RWRed6FdImp0PGduGf4cCJdsO5sRuovojn1yv+FUgHJBJQVCT71Rv3+b");

/// Whether a missing or unverifiable signature ABORTS the update.
///
/// Shipped false first (v8.5.3 to v8.6.0) because verification going live and
/// the signing pipeline going live cannot be made simultaneous across an
/// already-installed user base, and a wrong CI side would have bricked the
/// update path for everyone at once. Flipped 2026-09-07 after three
/// consecutive real releases logged `Update signature verified` on an
/// installed client and the live manifest carried a signature. From here an
/// unsigned bundle, a bundle with no published hash, or a build with no
/// pinned key all abort. See Brain runbook StreamNook_Update_Signing.
const ENFORCE_UPDATE_SIGNATURE: bool = true;

/// Verify a downloaded bundle before anything is unpacked or executed.
///
/// A signature that is PRESENT and WRONG always aborts, in both modes. That is
/// not a configuration question: it means someone tampered with the bundle or
/// the key rotated without the client knowing, and neither is survivable by
/// carrying on. Only ABSENT verification material is downgraded to a warning,
/// and only while `ENFORCE_UPDATE_SIGNATURE` is false.
fn verify_update_bundle(
    bytes: &[u8],
    expected_sha256: Option<&str>,
    signature: Option<&str>,
) -> Result<(), String> {
    // Hash: corruption check. Always fatal on mismatch.
    match expected_sha256 {
        Some(expected) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            let got: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            if !got.eq_ignore_ascii_case(expected.trim()) {
                return Err(format!(
                    "Update integrity check failed (expected {}, got {}). Aborting.",
                    expected, got
                ));
            }
        }
        None if ENFORCE_UPDATE_SIGNATURE => {
            return Err("Update has no published hash. Aborting.".to_string());
        }
        None => log::warn!("Update has no published SHA-256; integrity not verified"),
    }

    // Signature: authenticity check. This is the part a hostile update host
    // cannot satisfy.
    match (UPDATE_PUBKEY, signature) {
        (Some(pubkey), Some(sig)) => {
            crate::plugin_host::signing::verify_minisign(bytes, sig, pubkey)
                .map_err(|e| format!("Update signature verification failed: {e}. Aborting."))?;
            log::info!("Update signature verified");
        }
        (Some(_), None) if ENFORCE_UPDATE_SIGNATURE => {
            return Err("Update is unsigned. Aborting.".to_string());
        }
        (Some(_), None) => {
            log::warn!("Update is unsigned; accepting it because enforcement is off")
        }
        (None, _) if ENFORCE_UPDATE_SIGNATURE => {
            // Refuse rather than silently accept anything: enforcement on with no
            // pinned key is a build mistake, and treating it as "allow" would be
            // the exact opt-in-verification bug this replaces.
            return Err("Update signing key is not configured in this build. Aborting.".to_string());
        }
        (None, _) => log::warn!("No pinned update key in this build; signature not checked"),
    }

    Ok(())
}

#[cfg(test)]
mod update_manifest_tests {
    use super::{UpdateArtifact, UpdateManifest};

    /// A manifest as the pipeline wrote it BEFORE per-platform builds existed:
    /// flat fields only, describing a Windows bundle.
    fn legacy_manifest() -> UpdateManifest {
        serde_json::from_str(
            r#"{
                "version": "8.6.1",
                "download_url": "https://streamnook.app/d/StreamNook.7z",
                "bundle_name": "StreamNook.7z",
                "sha256": "abc123",
                "signature": "RWQlegacysig",
                "size": 12345
            }"#,
        )
        .expect("legacy manifest must still parse")
    }

    fn modern_manifest() -> UpdateManifest {
        serde_json::from_str(
            r#"{
                "version": "8.7.0",
                "download_url": "https://streamnook.app/d/StreamNook.7z",
                "bundle_name": "StreamNook.7z",
                "sha256": "winhash",
                "size": 111,
                "platforms": {
                    "windows-x86_64": {
                        "download_url": "https://streamnook.app/d/win/StreamNook.7z",
                        "bundle_name": "StreamNook.7z",
                        "sha256": "winhash2",
                        "size": 222
                    },
                    "macos-aarch64": {
                        "download_url": "https://streamnook.app/d/mac/StreamNook.app.tar.gz",
                        "bundle_name": "StreamNook.app.tar.gz",
                        "sha256": "machash",
                        "signature": "RWQmac",
                        "size": 333
                    }
                }
            }"#,
        )
        .expect("modern manifest parses")
    }

    #[test]
    fn a_legacy_manifest_still_updates_windows_clients() {
        // Every client already installed reads the flat fields. If this ever
        // returns None, that entire population is stranded on its current build.
        let a = legacy_manifest()
            .artifact_for_target("windows-x86_64")
            .expect("windows must resolve from the flat fields");
        assert_eq!(a.download_url, "https://streamnook.app/d/StreamNook.7z");
        assert_eq!(a.sha256.as_deref(), Some("abc123"));
        assert_eq!(a.size, Some(12345));
    }

    #[test]
    fn a_legacy_manifest_offers_macos_nothing_rather_than_a_windows_build() {
        // THE safety property. Falling back to the flat fields here would hand
        // a Mac StreamNook.7z and have it install a Windows build over itself.
        for target in ["macos-aarch64", "macos-x86_64", "linux-x86_64"] {
            assert!(
                legacy_manifest().artifact_for_target(target).is_none(),
                "{target} must get no artifact from a Windows-only manifest"
            );
        }
    }

    #[test]
    fn a_platform_entry_wins_over_the_legacy_fields() {
        let a = modern_manifest()
            .artifact_for_target("windows-x86_64")
            .expect("resolves");
        assert_eq!(a.download_url, "https://streamnook.app/d/win/StreamNook.7z");
        assert_eq!(
            a.sha256.as_deref(),
            Some("winhash2"),
            "the platforms entry must take precedence, not merge with the flat fields"
        );
    }

    #[test]
    fn macos_resolves_to_its_own_tarball_and_signature() {
        let a = modern_manifest()
            .artifact_for_target("macos-aarch64")
            .expect("macOS resolves");
        assert!(a.download_url.ends_with("StreamNook.app.tar.gz"));
        assert_eq!(a.sha256.as_deref(), Some("machash"));
        assert_eq!(
            a.signature.as_deref(),
            Some("RWQmac"),
            "each platform carries its OWN detached signature; a shared one              would not verify against a different artifact's bytes"
        );
    }

    #[test]
    fn an_unpublished_platform_gets_no_update() {
        assert!(modern_manifest()
            .artifact_for_target("linux-aarch64")
            .is_none());
    }

    #[test]
    fn the_key_space_matches_the_plugin_index() {
        // Both are built from std::env::consts, so a key that works for one
        // must work for the other. Drift here would be silent.
        let target = super::current_update_target();
        assert!(
            target.contains('-'),
            "target must be `<os>-<arch>`, got {target}"
        );
        let (os, arch) = target.split_once('-').unwrap();
        assert_eq!(os, std::env::consts::OS);
        assert_eq!(arch, std::env::consts::ARCH);
    }

    #[test]
    fn default_bundle_name_is_platform_appropriate() {
        let name = super::default_bundle_name();
        if cfg!(target_os = "macos") {
            assert_eq!(name, "StreamNook.app.tar.gz");
        } else if cfg!(target_os = "windows") {
            assert_eq!(name, "StreamNook.7z");
        } else if cfg!(target_os = "linux") {
            // Tied to the constant the SWAP step looks for, not to a second copy
            // of the literal: the download asking for one name while the swap
            // hunts for another is a silent "no update staged", with a successful
            // download and nothing installed.
            assert_eq!(name, super::LINUX_BUNDLE_NAME);
            assert_eq!(name, "StreamNook.AppImage");
        }
        assert!(!name.is_empty());
    }

    #[test]
    fn a_defaulted_artifact_carries_no_download_url() {
        // `unwrap_or_default()` at the call site must not fabricate a plausible
        // URL; an empty one cannot be downloaded by accident.
        assert!(UpdateArtifact::default().download_url.is_empty());
    }
}

#[cfg(test)]
mod update_verification_tests {
    use super::{verify_update_bundle, ENFORCE_UPDATE_SIGNATURE, UPDATE_PUBKEY};

    const BYTES: &[u8] = b"pretend this is StreamNook.7z";
    // sha256 of BYTES, lowercase hex.
    fn good_hash() -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(BYTES);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// A matching hash must clear the integrity step. With enforcement on the
    /// call still fails, but on the NEXT step (no signature), never on the hash.
    fn assert_hash_accepted(expected: &str) {
        match verify_update_bundle(BYTES, Some(expected), None) {
            Ok(()) => assert!(!ENFORCE_UPDATE_SIGNATURE, "enforcing must not accept an unsigned bundle"),
            Err(err) => {
                assert!(ENFORCE_UPDATE_SIGNATURE, "got: {err}");
                assert!(err.contains("unsigned"), "failed on the wrong step: {err}");
            }
        }
    }

    #[test]
    fn accepts_a_matching_hash() {
        assert_hash_accepted(&good_hash());
    }

    #[test]
    fn rejects_a_mismatched_hash_in_either_mode() {
        let bad = "0".repeat(64);
        let err = verify_update_bundle(BYTES, Some(&bad), None).unwrap_err();
        assert!(err.contains("integrity check failed"), "got: {err}");
    }

    #[test]
    fn hash_comparison_ignores_case_and_surrounding_space() {
        let padded = format!("  {}  ", good_hash().to_uppercase());
        assert_hash_accepted(&padded);
    }

    /// The regression this whole stage exists to prevent: verification must not
    /// be something the manifest can opt out of by omitting a field.
    #[test]
    fn unverifiable_bundles_are_refused_once_enforcement_is_on() {
        if !ENFORCE_UPDATE_SIGNATURE {
            // Enforcement is still off by design for the first release. Assert
            // the intended end state is reachable rather than silently passing:
            // with no pinned key, enforcement must refuse rather than allow.
            assert!(
                UPDATE_PUBKEY.is_none(),
                "a key is pinned, so ENFORCE_UPDATE_SIGNATURE should now be flipped to true",
            );
            return;
        }
        assert!(verify_update_bundle(BYTES, None, None).is_err(), "no hash must abort");
        assert!(
            verify_update_bundle(BYTES, Some(&good_hash()), None).is_err(),
            "an unsigned bundle must abort when enforcing",
        );
    }

    #[test]
    fn a_present_but_invalid_signature_always_aborts_when_a_key_is_pinned() {
        let Some(_) = UPDATE_PUBKEY else { return }; // nothing to check yet
        let err = verify_update_bundle(BYTES, Some(&good_hash()), Some("not-a-signature"))
            .unwrap_err();
        assert!(err.contains("signature"), "got: {err}");
    }
}

/// Check for updates via the self-hosted streamnook.app manifest (primary path).
async fn check_for_bundle_update_streamnook() -> Result<BundleUpdateStatus, String> {
    let client = reqwest::Client::builder()
        .user_agent("StreamNook")
        .build()
        .map_err(|e| e.to_string())?;

    let manifest: UpdateManifest = client
        .get(UPDATE_MANIFEST_URL)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch update manifest: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Update manifest returned an error: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse update manifest: {}", e))?;

    let current_version = get_current_app_version();

    // Resolve OUR platform's build first. A manifest that ships no build for
    // this platform is not an update, however new its version number is.
    let artifact = manifest.artifact_for_platform();
    let update_available =
        artifact.is_some() && remote_is_newer(&manifest.version, &current_version);
    if artifact.is_none() {
        log::info!(
            "[Update] manifest {} publishes no build for {}-{}; not offering an update",
            manifest.version,
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    let artifact = artifact.unwrap_or_default();

    let download_size = artifact
        .size
        .map(|s| format!("{:.1} MB", s as f64 / 1_048_576.0));

    Ok(BundleUpdateStatus {
        update_available,
        current_version: current_version.clone(),
        latest_version: manifest.version.clone(),
        download_url: Some(artifact.download_url.clone()),
        bundle_name: Some(
            artifact
                .bundle_name
                .clone()
                .unwrap_or_else(|| default_bundle_name().to_string()),
        ),
        download_size,
        component_changes: if update_available {
            Some(ComponentChanges {
                streamnook: Some(VersionChange {
                    from: current_version,
                    to: manifest.version.clone(),
                }),
                streamlink: None,
                ttvlol: None,
            })
        } else {
            None
        },
        release_notes: manifest.notes.clone(),
        sha256: artifact.sha256.clone(),
        signature: artifact.signature.clone(),
    })
}

/// Check for bundle updates via the self-hosted manifest.
///
/// The GitHub-release fallback that used to sit here was RETIRED, deliberately.
/// It published neither a hash nor a signature, so it was an unverified route to
/// code execution: the updater swaps StreamNook.exe and restarts, and that path
/// would install whatever it was handed. Keeping it as a fallback would also have
/// meant an attacker could DEGRADE to it simply by making the primary manifest
/// unreachable, which turns the signature requirement into a suggestion.
///
/// The cost is that updates pause while streamnook.app is unreachable. That is
/// the right trade: an update deferred by a few hours is a non-event, and the
/// manifest is served by Cloudflare, not the homelab.
///
/// This does NOT affect a user's FIRST download, which is a manual install and
/// never touches the updater. See `docs` / the update-signing runbook: first-run
/// trust is an Authenticode question, not a minisign one.
#[tauri::command]
pub async fn check_for_bundle_update() -> Result<BundleUpdateStatus, String> {
    check_for_bundle_update_streamnook().await
}

/// Legacy GitHub-release update check. RETIRED as an update source (see
/// `check_for_bundle_update`) and kept only because the component-version
/// bookkeeping below is still referenced elsewhere. It must never be reachable
/// from the install path again.
#[allow(dead_code)]
async fn check_for_bundle_update_github() -> Result<BundleUpdateStatus, String> {
    // Fetch remote version info
    let mut builder = reqwest::Client::builder().user_agent("StreamNook");

    // Inject PAT to bypass 60-req/hour limit during intense development
    if let Ok(token) = std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        builder = builder.default_headers(
            std::iter::once((
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
            ))
            .collect(),
        );
    }

    let client = builder.build().map_err(|e| e.to_string())?;

    // Directly download components.json from the latest release asset redirect
    // This entirely bypasses the api.github.com rate limit for unauthenticated users
    let remote: ComponentManifest = client
        .get("https://github.com/winters27/StreamNook/releases/latest/download/components.json")
        .send()
        .await
        .map_err(|e| format!("Failed to download remote components.json: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse remote components.json: {}", e))?;

    // The running binary's compiled-in version is the single source of truth for
    // "what's installed." We no longer consult the local components.json: the
    // exe-only bundle intentionally leaves it stale, so trusting it would falsely
    // report that an update is available.
    let current_version = get_current_app_version();
    let update_available = remote_is_newer(&remote.streamnook.version, &current_version);

    let mut status = BundleUpdateStatus {
        update_available,
        current_version: current_version.clone(),
        latest_version: remote.streamnook.version.clone(),
        download_url: None,
        bundle_name: None,
        download_size: None,
        component_changes: if update_available {
            Some(ComponentChanges {
                streamnook: Some(VersionChange {
                    from: current_version,
                    to: remote.streamnook.version.clone(),
                }),
                streamlink: None,
                ttvlol: None,
            })
        } else {
            None
        },
        release_notes: None,
        // The GitHub fallback publishes neither. Once ENFORCE_UPDATE_SIGNATURE
        // is on, this path stops being able to install, which is the intended
        // end state: it is currently an unverified route to code execution.
        // Until then it is reported, not silently trusted.
        sha256: None,
        signature: None,
    };

    // Set deterministic download URLs since we bypassed the API
    let download_url = format!(
        "https://github.com/winters27/StreamNook/releases/download/v{}/StreamNook.7z",
        remote.streamnook.version
    );
    status.bundle_name = Some("StreamNook.7z".to_string());
    status.download_url = Some(download_url.clone());

    // Fetch release notes strictly from raw CHANGELOG.md to bypass the API restrictions entirely.
    let changelog_url = format!(
        "https://raw.githubusercontent.com/winters27/StreamNook/v{}/CHANGELOG.md",
        remote.streamnook.version
    );

    if let Ok(changelog_res) = client.get(&changelog_url).send().await {
        if let Ok(changelog_text) = changelog_res.text().await {
            // Find the start of the version section
            let pattern = format!(
                r"(?s)## \[?{}\]?",
                regex::escape(&remote.streamnook.version)
            );
            if let Ok(re) = regex::Regex::new(&pattern) {
                if let Some(mat) = re.find(&changelog_text) {
                    let text_after = &changelog_text[mat.start()..];
                    // Slice until the next version block starts (denoted by a newline followed by "## ")
                    let end_idx = text_after[mat.len()..]
                        .find("\n## ")
                        .map(|i| i + mat.len())
                        .unwrap_or(text_after.len());
                    status.release_notes = Some(text_after[..end_idx].trim().to_string());
                }
            }
        }
    }

    // Optionally grab the download size using an HTTP HEAD request via redirects, skipping API data
    if let Ok(head_res) = client.head(&download_url).send().await {
        if let Some(content_length) = head_res.headers().get(reqwest::header::CONTENT_LENGTH) {
            if let Ok(len_str) = content_length.to_str() {
                if let Ok(size) = len_str.parse::<u64>() {
                    let mb = size as f64 / 1_048_576.0;
                    status.download_size = Some(format!("{:.1} MB", mb));
                }
            }
        }
    }

    Ok(status)
}

/// Legacy onboarding hook. Streamlink is no longer bundled or required, so there
/// is nothing to extract — kept as a no-op so the setup wizard's flow stays intact.
#[tauri::command]
pub async fn extract_bundled_components() -> Result<(), String> {
    Ok(())
}

/// Download and install bundle update
#[tauri::command]
pub async fn download_and_install_bundle(app_handle: tauri::AppHandle) -> Result<(), String> {
    let status = check_for_bundle_update().await?;
    if !status.update_available {
        return Err("No update available".to_string());
    }
    install_bundle_from_status(app_handle, status).await
}

/// Shared install body. Downloads and verifies the platform's bundle, then
/// stages it for `restart_to_apply_update`: on Windows the exe-only 7z is
/// extracted and a hardened batch script that swaps StreamNook.exe is written;
/// on macOS the `.app` tarball is unpacked and left in temp for a whole-bundle
/// swap.
async fn install_bundle_from_status(
    app_handle: tauri::AppHandle,
    status: BundleUpdateStatus,
) -> Result<(), String> {
    use tauri::Emitter;

    let download_url = status.download_url.ok_or("No download URL available")?;
    let bundle_name = status.bundle_name.ok_or("No bundle name available")?;

    // Emit progress
    let _ = app_handle.emit("bundle-update-progress", "Downloading bundle...");

    // Create temp directory
    let temp_dir = std::env::temp_dir().join("StreamNook-update");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;

    let bundle_path = temp_dir.join(&bundle_name);

    // Download the bundle
    let client = reqwest::Client::builder()
        .user_agent("StreamNook")
        .build()
        .map_err(|e| e.to_string())?;

    let mut response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download bundle: {}", e))?;

    // Stream the body in chunks so the UI can show real byte progress instead of
    // a single jump. Download maps to 0–90% of the bar; extract/install/complete
    // take it the rest of the way. If the server doesn't send a Content-Length
    // (chunked transfer) we can't compute a ratio, so the bar holds until the
    // quick post-download stages move it.
    let total = response.content_length();
    let mut bytes: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
    let mut downloaded: u64 = 0;
    let mut last_pct: u8 = u8::MAX;
    let _ = app_handle.emit("bundle-update-progress", "Downloading 0%");
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Failed to read bundle: {}", e))?
    {
        bytes.extend_from_slice(&chunk);
        downloaded += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = ((downloaded.min(total) * 90) / total) as u8;
            if pct != last_pct {
                last_pct = pct;
                let _ = app_handle.emit("bundle-update-progress", format!("Downloading {}%", pct));
            }
        }
    }

    verify_update_bundle(&bytes, status.sha256.as_deref(), status.signature.as_deref())?;

    std::fs::write(&bundle_path, &bytes).map_err(|e| format!("Failed to save bundle: {}", e))?;

    // macOS ships the update as a .app tarball, not a 7z, and installs it by
    // swapping the whole bundle. Until 8.6.3 this fell through to the 7z
    // extractor below and every macOS update died with "Failed to extract 7z
    // bundle" after a successful download, so no Mac could ever update
    // in-app. A runtime check rather than cfg so the branch compiles (and the
    // helpers stay testable) on every platform.
    if cfg!(target_os = "macos") {
        return stage_macos_bundle(&app_handle, &temp_dir, &bundle_path).await;
    }

    // Linux ships an AppImage: a single executable file, nothing to extract. It
    // had the SAME latent bug macOS carried until 8.6.3: it fell through to the
    // 7z extractor below and would have died with "Failed to extract 7z bundle"
    // on the first Linux release, so this is fixed before that release exists
    // rather than after.
    if cfg!(target_os = "linux") {
        return stage_linux_bundle(&app_handle, &temp_dir, &bundle_path).await;
    }

    let _ = app_handle.emit("bundle-update-progress", "Extracting bundle...");

    // Extract using native sevenz-rust library (no external 7z dependency)
    let extract_dir = temp_dir.join("extracted");
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract directory: {}", e))?;

    decompress_file(&bundle_path, &extract_dir)
        .map_err(|e| format!("Failed to extract 7z bundle: {}", e))?;

    let _ = app_handle.emit("bundle-update-progress", "Installing components...");

    // Manifest destination. (Streamlink is no longer bundled, so the updater
    // only swaps StreamNook.exe + components.json.)
    let dest_components = get_components_json_path()?;

    // Decide whether components.json copy happens HERE (no exe to swap, just a
    // component-only update) or DEFERRED into the batch script (exe swap is
    // needed; components.json must lag the exe so check_for_bundle_update's
    // local version never claims a version that isn't actually installed).
    let source_components = extract_dir.join("components.json");
    let source_exe = extract_dir.join("StreamNook.exe");

    if !source_exe.exists() && source_components.exists() {
        std::fs::copy(&source_components, &dest_components)
            .map_err(|e| format!("Failed to copy components.json: {}", e))?;
    }

    // Handle exe update - create batch script to replace and restart.
    // Hardening for v7.5.1: previous version ignored the `copy /y` errorlevel,
    // so an exe swap that silently failed (file lock, AV scan, etc.) left the
    // user on the OLD exe while components.json had already been overwritten
    // by the Rust side above — version reported as new while the running JS
    // was old. New batch:
    //   - retries the exe copy up to 5 times with 2-second backoff
    //   - only copies components.json AFTER the exe copy succeeds, so the
    //     two stay in lockstep
    //   - logs every step to %TEMP%\streamnook-update.log
    //   - on terminal failure, opens the extracted dir in Explorer and pops
    //     the log in Notepad so the user has a recovery path
    if source_exe.exists() {
        let current_exe = std::env::current_exe()
            .map_err(|e| format!("Failed to get current exe path: {}", e))?;

        let batch_script = format!(
            r#"@echo off
setlocal enabledelayedexpansion
set "SOURCE_EXE={source_exe}"
set "DEST_EXE={dest_exe}"
set "SOURCE_COMPONENTS={source_components}"
set "DEST_COMPONENTS={dest_components}"
set "TEMPDIR={tempdir}"
set "EXTRACTDIR={extractdir}"
set "LOG=%TEMP%\streamnook-update.log"
set "ERRFILE=%TEMP%\streamnook-update.err"

echo [%date% %time%] Update started > "%LOG%"
echo Source exe: %SOURCE_EXE% >> "%LOG%"
echo Dest exe: %DEST_EXE% >> "%LOG%"

:: Wait for StreamNook to close on its own, then force any stragglers. A second
:: or orphaned StreamNook.exe (a leftover from a crashed shutdown or an earlier
:: update attempt) used to spin this loop forever, so the swap never ran and the
:: app relaunched on the old version. Cap the graceful wait, then taskkill the
:: rest: every instance holds a lock on the shared exe image on disk, and we
:: relaunch a fresh one below regardless, so killing stale ones is safe.
set "WAITS=0"
:waitloop
tasklist /FI "IMAGENAME eq StreamNook.exe" 2>nul | find /I "StreamNook.exe" >nul
if errorlevel 1 goto closed
set /a WAITS+=1
echo [%date% %time%] Waiting for StreamNook to close (attempt !WAITS!) >> "%LOG%"
if !WAITS! GEQ 10 goto forceclose
timeout /t 1 /nobreak >nul 2>&1
goto waitloop

:forceclose
echo [%date% %time%] Still running after grace period; force-killing stragglers >> "%LOG%"
taskkill /f /im StreamNook.exe >nul 2>&1
timeout /t 2 /nobreak >nul 2>&1

:closed
echo [%date% %time%] StreamNook process closed, beginning exe copy >> "%LOG%"

set "ATTEMPTS=0"
:copyloop
copy /y "%SOURCE_EXE%" "%DEST_EXE%" >nul 2>"%ERRFILE%"
if not errorlevel 1 goto copysuccess

set /a ATTEMPTS+=1
echo [%date% %time%] Copy attempt !ATTEMPTS! failed: >> "%LOG%"
type "%ERRFILE%" >> "%LOG%" 2>nul
if !ATTEMPTS! GEQ 5 goto copyfailed
timeout /t 2 /nobreak >nul 2>&1
goto copyloop

:copysuccess
echo [%date% %time%] Exe copy succeeded after !ATTEMPTS! retries >> "%LOG%"

:: Now safe to bump components.json so it matches the installed exe
if exist "%SOURCE_COMPONENTS%" (
    copy /y "%SOURCE_COMPONENTS%" "%DEST_COMPONENTS%" >nul 2>"%ERRFILE%"
    if errorlevel 1 (
        echo [%date% %time%] WARNING: components.json copy failed but exe is installed >> "%LOG%"
        type "%ERRFILE%" >> "%LOG%" 2>nul
    ) else (
        echo [%date% %time%] components.json updated >> "%LOG%"
    )
)

echo [%date% %time%] Starting new exe >> "%LOG%"
start "" "%DEST_EXE%"

del "%ERRFILE%" >nul 2>&1
rd /s /q "%TEMPDIR%" >nul 2>&1
exit /b 0

:copyfailed
echo [%date% %time%] Update FAILED after 5 retries. >> "%LOG%"
echo [%date% %time%] Manually copy %SOURCE_EXE% to %DEST_EXE% to complete the update. >> "%LOG%"
echo [%date% %time%] components.json was NOT updated, so the app will continue to prompt for v{latest_version_for_log}. >> "%LOG%"

:: Surface the failure to the user. Explorer lands them in the extracted folder
:: where the new StreamNook.exe is sitting; Notepad shows them the log.
start "" "explorer.exe" "%EXTRACTDIR%"
start "" "notepad.exe" "%LOG%"

:: Restart the OLD exe so the user isn't left with no app open at all.
start "" "%DEST_EXE%"
del "%ERRFILE%" >nul 2>&1
exit /b 1
"#,
            source_exe = source_exe.to_string_lossy(),
            dest_exe = current_exe.to_string_lossy(),
            source_components = source_components.to_string_lossy(),
            dest_components = dest_components.to_string_lossy(),
            tempdir = temp_dir.to_string_lossy(),
            extractdir = extract_dir.to_string_lossy(),
            latest_version_for_log = status.latest_version,
        );

        let batch_path = temp_dir.join("update.bat");
        std::fs::write(&batch_path, batch_script)
            .map_err(|e| format!("Failed to write update script: {}", e))?;

        // Write the VBS launcher that restart_to_apply_update will run. It wraps
        // the batch and is launched via wscript with window style 0 (hidden).
        // wscript gives the batch a real (hidden) console so its
        // console-dependent commands (`timeout`, the `tasklist | find` wait
        // loop) run correctly. Do NOT spawn cmd directly with
        // CREATE_NO_WINDOW | DETACHED_PROCESS: the OS ignores CREATE_NO_WINDOW
        // when DETACHED_PROCESS is also set, which surfaces a visible console
        // and breaks the wait loop. A brief flash on creation is accepted in
        // exchange for a relaunch path that reliably completes.
        //
        // The launcher is only SPAWNED later, when the user clicks Restart (see
        // restart_to_apply_update). Staging stops here so the "Update Installed"
        // card can offer a manual restart instead of yanking the app closed.
        let vbs_script = format!(
            r#"Set WshShell = CreateObject("WScript.Shell")
WshShell.Run """{batch}""", 0, False
"#,
            batch = batch_path.to_string_lossy().replace("\\", "\\\\")
        );

        let vbs_path = temp_dir.join("update_launcher.vbs");
        std::fs::write(&vbs_path, vbs_script)
            .map_err(|e| format!("Failed to write VBS launcher: {}", e))?;

        // Leave the extracted exe + batch + vbs in temp for restart_to_apply_update
        // to consume; the batch cleans temp up itself once the swap succeeds.
        let _ = app_handle.emit("bundle-update-progress", "Update installed");

        return Ok(());
    }

    // Clean up temp directory
    let _ = std::fs::remove_dir_all(&temp_dir);

    let _ = app_handle.emit("bundle-update-progress", "Update complete!");

    Ok(())
}

/// macOS install: unpack the `.app` tarball into temp and leave it staged for
/// `restart_to_apply_update`.
///
/// `/usr/bin/tar` is part of the OS, so no archive crate is needed for the one
/// format the platform uses. The bundle is checked for its executable before
/// anything is staged: a stray archive would otherwise turn into an `rm -rf`
/// of the installed app followed by a `mv` of nothing.
async fn stage_macos_bundle(
    app_handle: &tauri::AppHandle,
    temp_dir: &Path,
    bundle_path: &Path,
) -> Result<(), String> {
    use tauri::Emitter;

    let _ = app_handle.emit("bundle-update-progress", "Extracting bundle...");
    let extract_dir = temp_dir.join("extracted");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract directory: {}", e))?;

    let output = tokio::process::Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(bundle_path)
        .arg("-C")
        .arg(&extract_dir)
        .output()
        .await
        .map_err(|e| format!("Failed to run tar: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to extract the app bundle: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let staged = extract_dir.join("StreamNook.app");
    let binary = staged.join("Contents").join("MacOS").join("StreamNook");
    if !binary.is_file() {
        return Err("The downloaded bundle does not contain StreamNook.app".to_string());
    }
    // tar restores the mode bits, and a bundle the app unpacked itself carries
    // no quarantine flag, so the relaunch does not hit Gatekeeper. Belt and
    // braces on the one bit that matters.
    crate::platform::fs::make_executable(&binary).map_err(|e| e.to_string())?;

    let _ = app_handle.emit("bundle-update-progress", "Update installed");
    Ok(())
}

/// Stage the downloaded AppImage so `restart_to_apply_update` can swap it in.
///
/// Much less work than the macOS twin because an AppImage is ONE FILE: there is
/// nothing to unpack. The two things that must happen are a sanity check that we
/// really received an AppImage (a 200-page HTML error from a misconfigured CDN
/// would otherwise be moved over the user's installed app and left unrunnable)
/// and the executable bit, which is the one attribute an AppImage cannot launch
/// without.
async fn stage_linux_bundle(
    app_handle: &tauri::AppHandle,
    temp_dir: &Path,
    bundle_path: &Path,
) -> Result<(), String> {
    use tauri::Emitter;

    let _ = app_handle.emit("bundle-update-progress", "Installing components...");

    let extract_dir = temp_dir.join("extracted");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create staging directory: {}", e))?;

    let bytes = std::fs::read(bundle_path)
        .map_err(|e| format!("Failed to read the downloaded bundle: {}", e))?;
    if !looks_like_an_appimage(&bytes) {
        return Err(
            "The downloaded file is not an AppImage; refusing to install it over the \
             running application"
                .to_string(),
        );
    }

    let staged = extract_dir.join(LINUX_BUNDLE_NAME);
    std::fs::copy(bundle_path, &staged)
        .map_err(|e| format!("Failed to stage the AppImage: {}", e))?;
    crate::platform::fs::make_executable(&staged).map_err(|e| e.to_string())?;

    let _ = app_handle.emit("bundle-update-progress", "Update installed");
    Ok(())
}

/// Filename the staged AppImage is parked under. Shared by the staging step and
/// the swap step so the two cannot disagree about where the file is.
const LINUX_BUNDLE_NAME: &str = "StreamNook.AppImage";

/// Does `bytes` start like a type-2 AppImage?
///
/// An AppImage is an ELF with the magic bytes `AI\x02` at offset 8, in the
/// padding of the ELF identification field. Checking both means a truncated
/// download, an HTML error page or a 7z served by mistake is refused BEFORE it
/// is moved over the installed app, which is the only point at which refusing is
/// still cheap.
///
/// Pure and taking a slice so every branch is testable from any host, the same
/// reason `artifact_for_target` takes its target explicitly.
fn looks_like_an_appimage(bytes: &[u8]) -> bool {
    bytes.len() > 11 && &bytes[0..4] == b"\x7fELF" && &bytes[8..11] == b"AI\x02"
}

/// The AppImage this process is running from.
///
/// **Must** come from `$APPIMAGE` and not `current_exe()`. Inside a running
/// AppImage the runtime mounts the payload read-only at `/tmp/.mount_XXXXXX/`
/// and execs the binary from THERE, so `current_exe()` points into a squashfs
/// mount that disappears on exit. A swap written against it would report success
/// into a temp directory, vanish, and leave the user on the old version with no
/// error anywhere, the same "compiles and does the wrong thing" class as the
/// `.bat` updater this module replaced.
///
/// `$APPIMAGE` is set by the AppImage runtime itself, so its absence means we are
/// not running as one (a `cargo run`, a `.deb` install, an extracted payload).
/// Refusing is right in every one of those cases: a `.deb` is updated by the
/// package manager, and swapping "the file this process came from" for a dev
/// build would overwrite the developer's own binary.
fn running_appimage() -> Result<PathBuf, String> {
    let raw = std::env::var_os("APPIMAGE").ok_or(
        "This build is not running as an AppImage ($APPIMAGE is unset), so there is \
         nothing to swap. Update through your package manager instead.",
    )?;
    let path = PathBuf::from(raw);
    if !path.is_file() {
        return Err(format!(
            "$APPIMAGE points at {}, which is not a file; refusing to swap",
            path.display()
        ));
    }
    Ok(path)
}

/// The `.app` that owns `exe`, if `exe` sits where a bundle keeps its main
/// executable (`Something.app/Contents/MacOS/<exe>`). Pure, so the shape can
/// be tested anywhere; `running_app_bundle` adds the filesystem check.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?;
    let contents = macos_dir.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && contents.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && bundle.extension().and_then(|e| e.to_str()) == Some("app");
    is_bundle.then(|| bundle.to_path_buf())
}

/// The `.app` this process runs from. Refuses anything else (a bare binary
/// under target/, a copied-out executable): swapping "the folder three levels
/// up" of an arbitrary path would delete an arbitrary folder.
fn running_app_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?;
    let bundle = bundle_of(&exe)
        .ok_or_else(|| format!("{} is not inside an .app bundle; refusing to swap", exe.display()))?;
    if !bundle.join("Contents").join("MacOS").is_dir() {
        return Err(format!("{} is not an .app bundle; refusing to swap", bundle.display()));
    }
    Ok(bundle)
}

/// `std::process::exit` skips Tauri's `RunEvent::Exit`, so the window-state
/// plugin would never auto-save (the relaunch forgets which monitor/size the
/// window had) and nothing debounced would reach disk. Flush it all by hand
/// before a hard exit, matching the flags the plugin is built with
/// (position/size/maximized only). The window-state plugin is desktop-only,
/// and no swap path runs on mobile (Android updates via the package installer).
fn flush_stores_before_hard_exit(app_handle: &tauri::AppHandle) {
    #[cfg(desktop)]
    {
        use tauri_plugin_window_state::{AppHandleExt, StateFlags};
        let _ = app_handle.save_window_state(
            StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED,
        );
    }
    #[cfg(not(desktop))]
    let _ = app_handle;
    let _ = crate::commands::settings::flush_settings_now();
    let _ = crate::services::universal_cache_service::flush_manifest_now();
    let _ = crate::services::mod_log_storage_service::ModLogStorageService::flush_now();
    let _ = crate::services::whisper_storage_service::WhisperStorageService::flush_now();
    let _ = crate::services::vod_progress_service::flush_now();
    let _ = crate::services::chat_logger_service::ChatLoggerService::flush_all();
}

/// Apply a staged bundle update by restarting StreamNook. download_and_install_bundle
/// leaves a hidden batch launcher in temp (Windows) or an unpacked `.app`
/// (macOS); this spawns the swap helper (which waits for this process to exit,
/// swaps the exe or the whole bundle, then relaunches) and exits. Called from
/// the "Restart StreamNook" button on the update-installed card, so the user
/// controls when the swap happens instead of it firing mid-install.
#[tauri::command]
pub async fn restart_to_apply_update(app_handle: tauri::AppHandle) -> Result<(), String> {
    let temp_dir = std::env::temp_dir().join("StreamNook-update");

    // In a `tauri dev` build, current_exe() is the dev binary under target/debug.
    // Running the swap batch would clobber the in-progress build, and even a
    // plain app_handle.restart() relaunches the bare exe — which tears down the
    // `tauri dev` server and its terminal. So in dev, do nothing here but discard
    // the staged bundle and return: the frontend reloads the webview instead,
    // which keeps the dev session alive and still re-runs the resume path.
    if cfg!(debug_assertions) {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Ok(());
    }

    // macOS: the .app that stage_macos_bundle unpacked. The helper script
    // waits for this pid to exit, removes the installed bundle, moves the new
    // one into its place and `open`s it (platform::app_update), so the exit
    // below is what lets it proceed, exactly like the Windows launcher.
    if cfg!(target_os = "macos") {
        let staged = temp_dir.join("extracted").join("StreamNook.app");
        if staged.is_dir() {
            let current = running_app_bundle()?;
            flush_stores_before_hard_exit(&app_handle);
            let request = crate::platform::app_update::SwapRequest {
                current: current.clone(),
                replacement: staged,
                pid: std::process::id(),
                relaunch: current,
            };
            crate::platform::app_update::swap_and_relaunch(&request)
                .map_err(|e| format!("Failed to start the bundle swap: {}", e))?;
            std::process::exit(0);
        }
    }

    // Linux: the AppImage stage_linux_bundle parked. Same helper, same wait-then-
    // swap-then-relaunch shape as macOS; only the artifact differs (one file
    // rather than a bundle directory), which `SwapFlavor::LinuxAppImage` encodes.
    if cfg!(target_os = "linux") {
        let staged = temp_dir.join("extracted").join(LINUX_BUNDLE_NAME);
        if staged.is_file() {
            let current = running_appimage()?;
            flush_stores_before_hard_exit(&app_handle);
            let request = crate::platform::app_update::SwapRequest {
                current: current.clone(),
                replacement: staged,
                pid: std::process::id(),
                relaunch: current,
            };
            crate::platform::app_update::swap_and_relaunch(&request)
                .map_err(|e| format!("Failed to start the AppImage swap: {}", e))?;
            std::process::exit(0);
        }
    }

    let vbs_path = temp_dir.join("update_launcher.vbs");

    if vbs_path.exists() {
        std::process::Command::new("wscript")
            .arg(&vbs_path)
            .spawn()
            .map_err(|e| format!("Failed to run update script: {}", e))?;
        flush_stores_before_hard_exit(&app_handle);
        std::process::exit(0);
    }

    // No exe-swap launcher staged (a component-only update already wrote its
    // files in place); a plain relaunch is enough to pick them up.
    app_handle.restart();
}

#[cfg(test)]
mod bundle_path_tests {
    use super::bundle_of;
    use std::path::{Path, PathBuf};

    #[test]
    fn the_main_executable_maps_to_its_bundle() {
        let exe = Path::new("/Applications/StreamNook.app/Contents/MacOS/StreamNook");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/StreamNook.app"))
        );
    }

    #[test]
    fn anything_that_is_not_a_bundle_executable_is_refused() {
        // A bare binary under target/: swapping three levels up would delete
        // the build tree.
        assert_eq!(
            bundle_of(Path::new("/Users/x/StreamNook/src-tauri/target/debug/StreamNook")),
            None
        );
        // Inside a bundle but not the MacOS executable.
        assert_eq!(
            bundle_of(Path::new("/Users/x/StreamNook.app/Contents/Resources/StreamNook")),
            None
        );
        // Right shape, wrong extension.
        assert_eq!(
            bundle_of(Path::new("/Users/x/StreamNook/Contents/MacOS/StreamNook")),
            None
        );
    }
}

#[cfg(test)]
mod appimage_tests {
    use super::{looks_like_an_appimage, running_appimage};

    /// The first 12 bytes of a type-2 AppImage: the ELF magic, then `AI\x02` in
    /// the ELF identification padding at offset 8.
    fn appimage_header() -> Vec<u8> {
        let mut v = b"\x7fELF\x02\x01\x01\x00".to_vec();
        v.extend_from_slice(b"AI\x02");
        v.push(0);
        v
    }

    #[test]
    fn a_real_appimage_header_is_accepted() {
        assert!(looks_like_an_appimage(&appimage_header()));
    }

    #[test]
    fn a_plain_elf_is_refused() {
        // A bare Linux binary IS an ELF but is not an AppImage. Moving one over
        // the installed app would leave something that starts and then cannot
        // find its payload.
        let mut v = b"\x7fELF\x02\x01\x01\x00".to_vec();
        v.extend_from_slice(&[0, 0, 0, 0]);
        assert!(!looks_like_an_appimage(&v));
    }

    #[test]
    fn the_things_a_broken_download_actually_delivers_are_refused() {
        // An HTML error page from a misconfigured CDN or an expired signed URL.
        assert!(!looks_like_an_appimage(b"<!DOCTYPE html><html><body>404"));
        // The Windows artifact, served by a manifest that named the wrong key.
        assert!(!looks_like_an_appimage(b"7z\xbc\xaf\x27\x1c\x00\x04\x00\x00\x00\x00"));
        // A download that died mid-header.
        assert!(!looks_like_an_appimage(b"\x7fELF"));
        assert!(!looks_like_an_appimage(b""));
    }

    /// `$APPIMAGE` is the ONLY correct source for the swap target. Inside a
    /// running AppImage `current_exe()` points into a read-only squashfs mount
    /// under /tmp that disappears on exit, so a swap written against it would
    /// succeed into nowhere.
    #[test]
    fn a_build_that_is_not_an_appimage_refuses_to_swap() {
        // The test process is not an AppImage, so this is the real unset case
        // rather than a simulated one. Asserting on the message because it is
        // what the user sees and it has to name the alternative.
        if std::env::var_os("APPIMAGE").is_none() {
            let err = running_appimage().expect_err("must refuse when $APPIMAGE is unset");
            assert!(
                err.contains("$APPIMAGE is unset"),
                "the refusal must say WHY, got: {err}"
            );
        }
    }

}
