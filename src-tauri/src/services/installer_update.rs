//! Windows: the NSIS install, its updates, and the move from the old portable exe.
//!
//! StreamNook used to ship as a bare `StreamNook.exe` in a 7z that updated itself
//! by swapping the exe (`commands::components`). It now installs per user with
//! Tauri's NSIS installer (`%LOCALAPPDATA%\StreamNook`) and updates through
//! `tauri-plugin-updater`: a signed installer is downloaded, checked against the
//! key in `tauri.windows.conf.json`, and run. That is the SAME minisign key the
//! exe-swap updater pins (`components::UPDATE_PUBKEY`), written the way the
//! plugin reads it; a test holds the two together.
//!
//! Three kinds of copy run this code:
//!
//! - **Installed:** updates through the plugin, which runs the new installer with
//!   `/UPDATE` (keeps shortcuts and data) and exits.
//! - **Portable, nothing installed:** the plugin is asked with a `>=` comparator,
//!   so the installer of the SAME version is offered as the normal update. It is
//!   run WITHOUT `/UPDATE`, because the template skips creating the Start Menu
//!   and desktop shortcuts in update mode, and a first install needs both.
//! - **Portable, an installed copy exists:** `startup` forwards to the installed
//!   exe and exits before anything else runs. The portable exe left behind by
//!   the move is therefore a launcher, so old shortcuts and pins keep working.
//!
//! `startup` also refuses (with a question) to run a build older than the
//! newest one that has run on this machine: an older build saving data written
//! by a newer one drops every settings field it does not know.
//!
//! The plugin path stays off until a real public key is configured
//! (`plugin_ready`), and any error on it falls back to the exe-swap updater, so a
//! mistake in the plugin's setup can never strand a copy. That covers the check
//! failing, and also a download that can never succeed (a signature the key
//! rejects, a missing installer): after one of those, the rest of the session
//! checks the exe-swap manifest, which can then carry a fixed build.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tauri::{Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::rt::AppHandle;

/// The placeholder in `tauri.windows.conf.json` until the real key is set.
const PUBKEY_PLACEHOLDER: &str = "STREAMNOOK_UPDATER_PUBKEY_NOT_SET";
/// `Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}` in
/// Tauri's NSIS template, under HKCU for a per-user install.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\StreamNook";
/// The app's identifier, which the installer stamps on its shortcuts as their
/// AppUserModelID. Pinned to tauri.conf.json by a test.
const APP_ID: &str = "com.streamnook.dev";
const MAIN_EXE: &str = "StreamNook.exe";
/// Highest version that has run against this machine's data, in the data root.
const VERSION_STAMP: &str = "last_run_version";

/// How this copy of StreamNook is installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// Running from the folder the installer registered.
    Installed,
    /// Running from anywhere else. `installed_exe` is the installed copy, when
    /// one is registered AND its exe is still there.
    Portable { installed_exe: Option<PathBuf> },
}

/// The folder the installer registered, from the uninstall entry. The template
/// writes `InstallLocation` wrapped in quotes.
fn registered_install_dir() -> Option<PathBuf> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let key = HSTRING::from(UNINSTALL_KEY);
    let value = HSTRING::from("InstallLocation");
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its byte length describe the same allocation;
    // RegGetValueW writes at most `bytes` bytes and NUL-terminates REG_SZ.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &value,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let len = (bytes as usize / 2).saturating_sub(1);
    let raw = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
    let trimmed = raw.trim().trim_matches('"').trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// Paths compared the way Windows compares them: resolved, case-insensitive.
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        dunce_like(std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    norm(a) == norm(b)
}

/// `canonicalize` on Windows returns `\\?\C:\...`; strip the verbatim prefix so
/// it compares equal to a plain path.
fn dunce_like(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p,
    }
}

fn classify(exe_dir: &Path, registered: Option<&Path>) -> InstallKind {
    match registered {
        Some(dir) if same_dir(exe_dir, dir) => InstallKind::Installed,
        Some(dir) => {
            let exe = dir.join(MAIN_EXE);
            InstallKind::Portable { installed_exe: exe.is_file().then_some(exe) }
        }
        None => InstallKind::Portable { installed_exe: None },
    }
}

pub fn install_kind() -> InstallKind {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    match exe_dir {
        Some(dir) => classify(&dir, registered_install_dir().as_deref()),
        None => InstallKind::Portable { installed_exe: None },
    }
}

fn message_box(text: &str, yes_no: bool) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_OK, MB_YESNO,
    };
    let style = if yes_no { MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 } else { MB_OK | MB_ICONWARNING };
    // SAFETY: owned HSTRINGs outlive the call; no parent window.
    let answer = unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("StreamNook"), style) };
    answer == IDYES
}

fn newer(a: &str, b: &str) -> bool {
    use crate::commands::components::parse_version;
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => {
            let n = x.len().max(y.len());
            (0..n)
                .map(|i| (x.get(i).copied().unwrap_or(0), y.get(i).copied().unwrap_or(0)))
                .find(|(p, q)| p != q)
                .is_some_and(|(p, q)| p > q)
        }
        _ => false,
    }
}

/// What to do with this run, given the stamp left by earlier runs.
#[derive(Debug, PartialEq, Eq)]
enum StampVerdict {
    /// Same or newer than anything before: run, and record this version.
    Run { write: bool },
    /// Older than a build that has already run here.
    Older { newest: String },
}

fn judge(stamp: Option<&str>, current: &str) -> StampVerdict {
    match stamp.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) if newer(s, current) => StampVerdict::Older { newest: s.to_string() },
        Some(s) => StampVerdict::Run { write: s != current },
        None => StampVerdict::Run { write: true },
    }
}

fn guard_against_downgrade() {
    let Ok(root) = crate::services::cache_service::get_app_data_dir() else { return };
    let path = root.join(VERSION_STAMP);
    let current = env!("CARGO_PKG_VERSION");
    let stamp = std::fs::read_to_string(&path).ok();
    match judge(stamp.as_deref(), current) {
        StampVerdict::Run { write } => {
            if write {
                let _ = std::fs::write(&path, current);
            }
        }
        StampVerdict::Older { newest } => {
            let text = format!(
                "StreamNook {newest} has already run on this computer, and this copy is the older {current}.\n\n\
                 Running an older version can lose settings the newer one saved. To get the newer version, \
                 open StreamNook from the Start menu or download it from streamnook.app.\n\n\
                 Run this older copy anyway?"
            );
            if !message_box(&text, true) {
                std::process::exit(0);
            }
        }
    }
}

/// First thing on Windows, before Tauri builds anything (so before the
/// single-instance plugin, deep-link registration and WebView2):
///
/// 1. A portable copy with an installed copy beside it hands over and exits.
///    If the installed exe will not start, this one does NOT run instead: it
///    says so and exits, because running an older build over the installed
///    one's data loses settings.
/// 2. The installed copy takes the AppUserModelID its shortcuts carry, so a
///    window opened through the old portable launcher groups with Start Menu
///    pins.
/// 3. A build older than one that already ran here asks before running.
///
/// Release builds only: a `tauri dev` binary must never hand itself over to the
/// installed app, and debug builds keep their data in their own folder.
pub fn startup() {
    if cfg!(debug_assertions) {
        return;
    }
    match install_kind() {
        InstallKind::Portable { installed_exe: Some(exe) } => {
            let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
            match std::process::Command::new(&exe).args(&args).spawn() {
                Ok(_) => std::process::exit(0),
                Err(e) => {
                    message_box(
                        &format!(
                            "StreamNook is installed in {}, but it could not be started ({e}).\n\n\
                             Open it from the Start menu, or reinstall it from streamnook.app.",
                            exe.parent().map(|p| p.display().to_string()).unwrap_or_default()
                        ),
                        false,
                    );
                    std::process::exit(1);
                }
            }
        }
        InstallKind::Installed => {
            use windows::core::HSTRING;
            // SAFETY: called before any window exists, as the API requires.
            let _ = unsafe {
                windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(APP_ID))
            };
        }
        InstallKind::Portable { installed_exe: None } => {}
    }
    guard_against_downgrade();
}

/// Set when a plugin download or install failed in a way retrying cannot fix.
static PLUGIN_REFUSED: AtomicBool = AtomicBool::new(false);

/// Failures that mean the published update itself is wrong for this copy: the
/// signature does not verify against the pinned key or names another version,
/// the installer is not there (4xx), or it is not an installer. Network drops,
/// timeouts and 5xx are worth retrying on the plugin path.
fn is_permanent(e: &tauri_plugin_updater::Error) -> bool {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion
        | E::InvalidUpdaterFormat => true,
        // The plugin's only Network error: "Download request failed with status: 404 Not Found".
        E::Network(msg) => msg
            .split("status: ")
            .nth(1)
            .and_then(|s| s.get(..3))
            .and_then(|code| code.parse::<u16>().ok())
            .is_some_and(|code| (400..500).contains(&code)),
        _ => false,
    }
}

fn note_failure(e: &tauri_plugin_updater::Error) {
    if is_permanent(e) && !PLUGIN_REFUSED.swap(true, Ordering::Relaxed) {
        log::warn!("[Update] the installer update cannot be used ({e}); checking the exe-swap manifest until restart");
    }
}

/// The plugin is configured with a real key and has not been refused this
/// session. Otherwise every check uses the exe-swap updater.
pub fn plugin_usable(app: &AppHandle) -> bool {
    plugin_ready(app) && !PLUGIN_REFUSED.load(Ordering::Relaxed)
}

/// The plugin is configured with a real key.
fn plugin_ready(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty() && k != PUBKEY_PLACEHOLDER)
}

/// The update the last plugin check found, and its installer once downloaded.
struct Staged {
    update: Update,
    bytes: Option<Vec<u8>>,
    /// Installing over a portable copy: run the installer without `/UPDATE`.
    first_install: bool,
}

static STAGED: Lazy<Mutex<Option<Staged>>> = Lazy::new(|| Mutex::new(None));

fn staged() -> std::sync::MutexGuard<'static, Option<Staged>> {
    STAGED.lock().unwrap_or_else(|e| e.into_inner())
}

/// True when the last check was answered by the plugin, so download and apply
/// must go through it too.
pub fn has_pending() -> bool {
    staged().is_some()
}

/// Ask the plugin. `Ok(None)` = up to date. A portable copy is offered the
/// installer of its own version (that is the move to the installed app).
pub async fn check(app: &AppHandle) -> Result<Option<Update>, String> {
    // A failed check must not leave an older answer staged.
    *staged() = None;
    let first_install = matches!(install_kind(), InstallKind::Portable { .. });
    let flush = app.clone();
    let mut builder = app.updater_builder().on_before_exit(move || {
        crate::commands::components::flush_stores_before_hard_exit(&flush);
        flush.cleanup_before_exit();
    });
    if first_install {
        builder = builder.version_comparator(|current, release| release.version >= current);
    }
    let updater = builder.build().map_err(|e| format!("updater setup: {e}"))?;
    let found = updater.check().await.map_err(|e| format!("update check: {e}"))?;
    *staged() = found.clone().map(|update| Staged { update, bytes: None, first_install });
    Ok(found)
}

/// Download and verify the staged update, reporting progress on the same
/// `bundle-update-progress` channel the exe-swap path uses.
pub async fn download(app: &AppHandle) -> Result<(), String> {
    let update = staged().as_ref().map(|s| s.update.clone()).ok_or("No update staged")?;
    let _ = app.emit("bundle-update-progress", "Downloading 0%");
    let progress = app.clone();
    let mut downloaded: u64 = 0;
    let mut last_pct = u8::MAX;
    let bytes = update
        .download(
            move |chunk, total| {
                downloaded += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let pct = ((downloaded.min(total) * 90) / total) as u8;
                    if pct != last_pct {
                        last_pct = pct;
                        let _ = progress.emit("bundle-update-progress", format!("Downloading {pct}%"));
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| {
            note_failure(&e);
            if is_permanent(&e) {
                *staged() = None;
            }
            format!("Update download failed: {e}")
        })?;
    if let Some(s) = staged().as_mut() {
        s.bytes = Some(bytes);
    }
    let _ = app.emit("bundle-update-progress", "Update installed");
    Ok(())
}

/// Run the downloaded installer and exit. Returns Ok(false) when nothing is
/// staged, so the caller can fall through to the exe-swap path.
pub fn apply(app: &AppHandle) -> Result<bool, String> {
    let Some(Staged { update, bytes: Some(bytes), first_install }) = staged().take() else {
        return Ok(false);
    };
    // The installer, and the app it relaunches, inherit this environment. A
    // remote-debugging port is a per-launch opt-in, and the relaunch starts
    // while this process's WebView2 can still hold that port, which hangs the
    // new instance before its first window.
    std::env::remove_var("SN_CDP_PORT");
    if !first_install {
        crate::services::app_exit::record("update: running the new installer");
        // Runs `on_before_exit` (flush), starts the installer with
        // /P /UPDATE /R and exits the process.
        update.install(&bytes).map_err(|e| {
            note_failure(&e);
            format!("Failed to start the installer: {e}")
        })?;
        return Ok(true);
    }
    // First install over a portable copy: no /UPDATE, or the installer would
    // skip the Start Menu and desktop shortcuts. /P = small progress window,
    // /R = start the installed app when done.
    let dir = std::env::temp_dir().join("StreamNook-update");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp directory: {e}"))?;
    let installer = dir.join("StreamNook-Setup.exe");
    std::fs::write(&installer, &bytes).map_err(|e| format!("Failed to save the installer: {e}"))?;
    crate::services::app_exit::record("update: installing StreamNook in place of the portable copy");
    crate::commands::components::flush_stores_before_hard_exit(app);
    std::process::Command::new(&installer)
        .args(["/P", "/R"])
        .spawn()
        .map_err(|e| format!("Failed to start the installer: {e}"))?;
    app.cleanup_before_exit();
    crate::services::app_exit::process_exit(0, "update: installer started over the portable copy");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_matches_the_bundle_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(conf["identifier"].as_str(), Some(APP_ID));
    }

    #[test]
    fn the_windows_config_points_the_plugin_at_the_tauri_manifest() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.windows.conf.json")).expect("tauri.windows.conf.json");
        let updater = &conf["plugins"]["updater"];
        assert_eq!(updater["endpoints"][0].as_str(), Some("https://streamnook.app/api/v1/update/tauri"));
        assert_eq!(updater["windows"]["installMode"].as_str(), Some("passive"));
        assert_eq!(conf["bundle"]["windows"]["nsis"]["installMode"].as_str(), Some("currentUser"));
    }

    /// The release workflow signs the setup.exe with minisign and writes the
    /// version into the signature's trusted comment, so installed copies can
    /// insist on it: a tampered manifest cannot pair a new version number with
    /// an older signed installer. Tauri's own signer stays off; it would demand
    /// a second, password-protected copy of the key on every Windows build.
    #[test]
    fn installers_must_be_signed_for_their_version_by_the_workflow() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.windows.conf.json")).expect("tauri.windows.conf.json");
        assert_eq!(conf["plugins"]["updater"]["requireSignedVersion"].as_bool(), Some(true));
        assert_ne!(conf["bundle"]["createUpdaterArtifacts"].as_bool(), Some(true));
    }

    #[test]
    fn only_failures_a_retry_cannot_fix_refuse_the_plugin() {
        use tauri_plugin_updater::Error as E;
        let status = |s: &str| E::Network(format!("Download request failed with status: {s}"));
        assert!(is_permanent(&E::MissingSignedVersion));
        assert!(is_permanent(&E::SignedVersionMismatch { signed: "8.9.0".into(), announced: "8.9.1".into() }));
        assert!(is_permanent(&E::InvalidUpdaterFormat));
        assert!(is_permanent(&status("404 Not Found")));
        assert!(is_permanent(&status("403 Forbidden")));
        assert!(!is_permanent(&status("503 Service Unavailable")));
        assert!(!is_permanent(&E::Network("garbled".into())));
        assert!(!is_permanent(&E::Io(std::io::Error::other("reset"))));
    }

    /// Both updaters trust ONE key: the installer updater's public key is the
    /// minisign key the exe-swap updater pins (`UPDATE_PUBKEY`, 3C746A225DA1775E),
    /// written the way the plugin reads it (base64 of the whole .pub file).
    #[test]
    fn the_installer_updater_trusts_the_same_key_as_the_exe_swap_updater() {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.windows.conf.json")).expect("tauri.windows.conf.json");
        let encoded = conf["plugins"]["updater"]["pubkey"].as_str().expect("pubkey");
        let file = String::from_utf8(STANDARD.decode(encoded).expect("base64")).expect("utf8");
        let mut lines = file.lines();
        assert!(lines.next().is_some_and(|c| c.starts_with("untrusted comment:")));
        assert_eq!(lines.next(), crate::commands::components::UPDATE_PUBKEY);
    }

    #[test]
    fn classify_reads_the_registered_folder() {
        let tmp = std::env::temp_dir().join("sn-installer-classify");
        let installed = tmp.join("Installed");
        let portable = tmp.join("Portable");
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::create_dir_all(&portable).unwrap();

        assert_eq!(classify(&portable, None), InstallKind::Portable { installed_exe: None });
        // Registered, but the exe is gone: run as portable (the stamp guards data).
        assert_eq!(classify(&portable, Some(&installed)), InstallKind::Portable { installed_exe: None });
        std::fs::write(installed.join(MAIN_EXE), b"").unwrap();
        assert_eq!(
            classify(&portable, Some(&installed)),
            InstallKind::Portable { installed_exe: Some(installed.join(MAIN_EXE)) }
        );
        // Case and a trailing separator do not make the installed copy portable.
        let shouted = PathBuf::from(format!("{}\\", installed.display().to_string().to_uppercase()));
        assert_eq!(classify(&installed, Some(&shouted)), InstallKind::Installed);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn verbatim_prefix_is_stripped_but_unc_is_kept() {
        assert_eq!(dunce_like(PathBuf::from(r"\\?\C:\x")), PathBuf::from(r"C:\x"));
        assert_eq!(dunce_like(PathBuf::from(r"\\?\UNC\srv\x")), PathBuf::from(r"\\?\UNC\srv\x"));
    }

    #[test]
    fn the_stamp_only_ever_moves_forward() {
        assert_eq!(judge(None, "8.9.0"), StampVerdict::Run { write: true });
        assert_eq!(judge(Some("8.9.0"), "8.9.0"), StampVerdict::Run { write: false });
        assert_eq!(judge(Some("8.8.11\n"), "8.9.0"), StampVerdict::Run { write: true });
        assert_eq!(judge(Some("8.10.0"), "8.9.0"), StampVerdict::Older { newest: "8.10.0".into() });
        // Unreadable stamps never block a launch.
        assert_eq!(judge(Some("garbage"), "8.9.0"), StampVerdict::Run { write: true });
    }
}
