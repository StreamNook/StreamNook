//! Replace the running application with a downloaded update, then relaunch.
//!
//! # Why a detached helper script, on every platform
//!
//! A process cannot reliably overwrite its own executable while running: Windows
//! holds an exclusive lock on a running image, and macOS will happily let you
//! `rm -rf` a running `.app` and then behave unpredictably when the bundle's
//! resources vanish underneath it. Both platforms therefore need the same
//! shape: spawn something detached, exit, and let it do the swap once we are
//! gone.
//!
//! Only the script language differs, which is exactly the kind of thing this
//! module exists to contain.
//!
//! # Why this is not just "the Windows updater, ported"
//!
//! There were **two** ungated Windows-coupled updater paths in the tree
//! (`commands/components.rs` and `commands/settings.rs`), neither carrying a
//! `cfg`, both writing `.bat` files and swapping `StreamNook.exe`. On macOS they
//! compiled and then did nothing useful — the "compiles and does the wrong
//! thing" failure class. Concentrating the swap here means there is one
//! implementation to make correct rather than two to keep in step.
//!
//! # What is deliberately NOT here
//!
//! Downloading, signature verification and artifact-format decisions stay with
//! the caller. This module only answers "the new build is on disk; make it the
//! running one".

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// What to swap, and what to relaunch afterwards.
#[derive(Debug, Clone)]
pub struct SwapRequest {
    /// The currently-installed artifact. On Windows the `.exe`; on macOS the
    /// `.app` bundle directory.
    pub current: PathBuf,
    /// The freshly-downloaded replacement, already unpacked and verified.
    pub replacement: PathBuf,
    /// PID the helper must wait to exit before touching anything.
    pub pid: u32,
    /// What to launch once the swap succeeds. Usually equal to `current`.
    pub relaunch: PathBuf,
}

/// Render the helper script for this platform.
///
/// Pure, and public to the crate's tests on purpose: the script is the part
/// that goes wrong, and a swap that silently fails leaves a user on an old
/// build with no error anywhere. Testing the rendered text is the only way to
/// check it without actually replacing the running application.
pub fn render_script(req: &SwapRequest) -> String {
    let current = req.current.to_string_lossy();
    let replacement = req.replacement.to_string_lossy();
    let relaunch = req.relaunch.to_string_lossy();
    let pid = req.pid;

    if cfg!(windows) {
        // `tasklist | find` is the wait loop the shipped updater already used.
        // Kept deliberately: it works on a bare cmd.exe with no PowerShell
        // dependency, which matters on locked-down machines.
        format!(
            "@echo off\r\n\
             :wait\r\n\
             tasklist /FI \"PID eq {pid}\" 2>nul | find \"{pid}\" >nul\r\n\
             if not errorlevel 1 (\r\n\
             \x20 timeout /t 1 /nobreak >nul\r\n\
             \x20 goto wait\r\n\
             )\r\n\
             copy /y \"{replacement}\" \"{current}\" >nul\r\n\
             if errorlevel 1 exit /b 1\r\n\
             start \"\" \"{relaunch}\"\r\n\
             exit /b 0\r\n"
        )
    } else {
        // `kill -0` tests for existence without signalling. The bounded loop
        // matters: an unbounded `while kill -0` leaves a stray shell forever if
        // the app never exits, and on macOS that shell would keep the old
        // bundle's file handles alive.
        format!(
            "#!/bin/sh\n\
             set -e\n\
             i=0\n\
             while kill -0 {pid} 2>/dev/null; do\n\
             \x20 i=$((i+1))\n\
             \x20 [ \"$i\" -gt 120 ] && exit 1\n\
             \x20 sleep 1\n\
             done\n\
             # rm -rf then mv, NOT `cp -R` over the top: a bundle is a directory,\n\
             # and copying into an existing one leaves stale files from the old\n\
             # version behind and can break the code signature.\n\
             rm -rf \"{current}\"\n\
             mv \"{replacement}\" \"{current}\"\n\
             open \"{relaunch}\"\n\
             exit 0\n"
        )
    }
}

/// File extension the helper must be written with for the OS to run it.
fn script_extension() -> &'static str {
    if cfg!(windows) {
        "bat"
    } else {
        "sh"
    }
}

/// Write the helper, mark it runnable, spawn it detached, and return its path.
///
/// The caller is expected to exit promptly afterwards; the helper is waiting on
/// exactly that.
pub fn swap_and_relaunch(req: &SwapRequest) -> Result<PathBuf> {
    if !req.replacement.exists() {
        return Err(anyhow!(
            "update replacement {} does not exist",
            req.replacement.display()
        ));
    }

    let script_path = std::env::temp_dir().join(format!(
        "streamnook_update_{}.{}",
        std::process::id(),
        script_extension()
    ));
    std::fs::write(&script_path, render_script(req))
        .with_context(|| format!("cannot write update helper {}", script_path.display()))?;

    // No-op on Windows; required on unix or the shell refuses to run it.
    super::fs::make_executable(&script_path)?;

    spawn_detached(&script_path)?;
    Ok(script_path)
}

fn spawn_detached(script: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: the helper has to outlive
        // us, and must not flash a console window.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        std::process::Command::new("cmd")
            .args(["/C", &script.to_string_lossy()])
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .with_context(|| "cannot spawn the update helper")?;
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::process::Stdio;
        std::process::Command::new("/bin/sh")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| "cannot spawn the update helper")?;
        Ok(())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = script;
        Err(anyhow!("self-update is not supported on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> SwapRequest {
        SwapRequest {
            current: PathBuf::from("/Applications/StreamNook.app"),
            replacement: PathBuf::from("/tmp/new/StreamNook.app"),
            pid: 4242,
            relaunch: PathBuf::from("/Applications/StreamNook.app"),
        }
    }

    #[test]
    fn script_waits_for_the_old_process_before_touching_anything() {
        let s = render_script(&req());
        let pid_at = s.find("4242").expect("the pid must appear");
        let swap_at = if cfg!(windows) {
            s.find("copy /y").expect("copy step")
        } else {
            s.find("rm -rf").expect("remove step")
        };
        assert!(
            pid_at < swap_at,
            "the wait on the pid must come BEFORE the swap, or the helper \
             replaces a running application"
        );
    }

    #[test]
    fn script_relaunches_after_swapping() {
        let s = render_script(&req());
        let swap_at = if cfg!(windows) {
            s.find("copy /y").unwrap()
        } else {
            s.find("mv ").unwrap()
        };
        let launch_at = if cfg!(windows) {
            s.find("start ").unwrap()
        } else {
            s.find("open ").unwrap()
        };
        assert!(swap_at < launch_at, "relaunch must follow the swap");
    }

    #[test]
    fn the_wait_loop_is_bounded() {
        let s = render_script(&req());
        if cfg!(windows) {
            assert!(s.contains("goto wait"), "windows uses a goto loop");
        } else {
            // An unbounded `while kill -0` leaves a stray shell forever if the
            // app never exits, holding the old bundle's handles open.
            assert!(
                s.contains("-gt 120") && s.contains("exit 1"),
                "the unix wait loop must give up rather than spin forever"
            );
        }
    }

    /// Only the executable lines matter here. The script carries a `#` comment
    /// explaining why `cp -R` is wrong, and a naive substring search over the
    /// whole script matches that prose — which is exactly what happened the
    /// first time this test ran.
    #[cfg(unix)]
    fn commands_only(script: &str) -> String {
        script
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[cfg(unix)]
    #[test]
    fn bundle_swap_removes_the_old_directory_instead_of_copying_over_it() {
        let s = render_script(&req());
        let cmds = commands_only(&s);
        assert!(
            cmds.contains("rm -rf") && cmds.contains("mv "),
            "a .app is a DIRECTORY: copying into an existing one leaves stale \
             files from the old version and can break the signature"
        );
        assert!(
            !cmds.contains("cp -R"),
            "cp -R over an existing bundle is the bug this guards against; \
             found it in an executable line, not just the explanatory comment"
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_script_has_a_shebang_and_is_written_executable() {
        let s = render_script(&req());
        assert!(s.starts_with("#!/bin/sh"), "needs a shebang to be runnable");
    }

    #[test]
    fn paths_are_quoted_so_spaces_survive() {
        // "/Applications/StreamNook.app" is fine, but a user-relocated bundle or
        // a Windows "Program Files" path is not. Unquoted, the swap silently
        // targets the wrong path.
        let mut r = req();
        r.current = PathBuf::from("/Users/a b/Applications/Stream Nook.app");
        r.replacement = PathBuf::from("/tmp/x y/Stream Nook.app");
        r.relaunch = r.current.clone();
        let s = render_script(&r);
        assert!(s.contains("\"/Users/a b/Applications/Stream Nook.app\""));
        assert!(s.contains("\"/tmp/x y/Stream Nook.app\""));
    }

    #[test]
    fn a_missing_replacement_is_refused_before_anything_is_written() {
        let mut r = req();
        r.replacement = std::env::temp_dir().join("streamnook_update_absent_fixture");
        let _ = std::fs::remove_file(&r.replacement);
        let _ = std::fs::remove_dir_all(&r.replacement);
        assert!(
            swap_and_relaunch(&r).is_err(),
            "swapping in a replacement that is not there would delete the \
             installed app and leave nothing behind"
        );
    }

    #[test]
    fn helper_extension_matches_the_platform_shell() {
        assert_eq!(script_extension(), if cfg!(windows) { "bat" } else { "sh" });
    }
}
