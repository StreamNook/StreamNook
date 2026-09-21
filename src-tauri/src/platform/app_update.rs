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

/// The swap shape a platform needs. Three, not two.
///
/// Until 8.6.5 this was a plain `cfg!(windows)` fork, so Linux inherited the
/// macOS branch wholesale and ended its script with `open`, which is macOS-only:
/// the swap succeeded and the app never came back. Naming the three shapes makes
/// the Linux one a case somebody has to write rather than a default it falls
/// into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapFlavor {
    /// A single `.exe`, locked while running. `copy /y` over it, `start` it.
    WindowsExe,
    /// A `.app` DIRECTORY. `rm -rf` then `mv`, relaunched through `open`.
    MacOsBundle,
    /// A single-file AppImage, executable in place and launched directly.
    LinuxAppImage,
}

/// The shape this build runs as.
///
/// A build for an unknown unix is treated as Linux rather than macOS: the
/// AppImage script uses only POSIX tools, so it degrades to "probably right"
/// instead of "calls a binary that does not exist".
pub fn current_flavor() -> SwapFlavor {
    if cfg!(windows) {
        SwapFlavor::WindowsExe
    } else if cfg!(target_os = "macos") {
        SwapFlavor::MacOsBundle
    } else {
        SwapFlavor::LinuxAppImage
    }
}

/// Render the helper script for the platform this build runs on.
pub fn render_script(req: &SwapRequest) -> String {
    render_script_for(req, current_flavor())
}

/// Render the helper script for an EXPLICIT flavor.
///
/// Pure, and public to the crate's tests on purpose: the script is the part
/// that goes wrong, and a swap that silently fails leaves a user on an old
/// build with no error anywhere. Testing the rendered text is the only way to
/// check it without actually replacing the running application.
///
/// The flavor is a parameter for the same reason `artifact_for_target` takes
/// one next door: with `cfg!` inside, each platform's script could only ever be
/// tested ON that platform, and the macOS and Linux scripts would be checked
/// exclusively by runners that do not exist yet. Here every host tests all
/// three.
pub fn render_script_for(req: &SwapRequest, flavor: SwapFlavor) -> String {
    let current = req.current.to_string_lossy();
    let replacement = req.replacement.to_string_lossy();
    let relaunch = req.relaunch.to_string_lossy();
    let pid = req.pid;

    // The unix wait loop is shared: `kill -0` tests for existence without
    // signalling, and the bound matters because an unbounded `while kill -0`
    // leaves a stray shell forever if the app never exits (on macOS that shell
    // would also keep the old bundle's file handles alive).
    let unix_wait = format!(
        "#!/bin/sh\n\
         set -e\n\
         i=0\n\
         while kill -0 {pid} 2>/dev/null; do\n\
         \x20 i=$((i+1))\n\
         \x20 [ \"$i\" -gt 120 ] && exit 1\n\
         \x20 sleep 1\n\
         done\n"
    );

    match flavor {
        SwapFlavor::WindowsExe => {
            // `tasklist | find` is the wait loop the shipped updater already
            // used. Kept deliberately: it works on a bare cmd.exe with no
            // PowerShell dependency, which matters on locked-down machines.
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
        }
        SwapFlavor::MacOsBundle => {
            format!(
                "{unix_wait}\
             # rm -rf then mv, NOT `cp -R` over the top: a bundle is a directory,\n\
             # and copying into an existing one leaves stale files from the old\n\
             # version behind and can break the code signature.\n\
             rm -rf \"{current}\"\n\
             mv \"{replacement}\" \"{current}\"\n\
             open \"{relaunch}\"\n\
             exit 0\n"
            )
        }
        SwapFlavor::LinuxAppImage => {
            format!(
                "{unix_wait}\
             # An AppImage is ONE FILE, so `mv -f` alone replaces it. Deliberately\n\
             # no `rm` first: with `set -e` a failed mv then leaves the old build\n\
             # exactly where it was, instead of deleting it and leaving nothing.\n\
             mv -f \"{replacement}\" \"{current}\"\n\
             chmod +x \"{current}\"\n\
             # NOT `open`: that is macOS. An AppImage is executed directly, and\n\
             # nohup + & detaches it so this helper can exit without taking the\n\
             # freshly-launched app down with it.\n\
             nohup \"{relaunch}\" >/dev/null 2>&1 &\n\
             exit 0\n"
            )
        }
    }
}

/// File extension the helper must be written with for the OS to run it.
fn script_extension_for(flavor: SwapFlavor) -> &'static str {
    match flavor {
        SwapFlavor::WindowsExe => "bat",
        SwapFlavor::MacOsBundle | SwapFlavor::LinuxAppImage => "sh",
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
        script_extension_for(current_flavor())
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

    const ALL: [SwapFlavor; 3] = [
        SwapFlavor::WindowsExe,
        SwapFlavor::MacOsBundle,
        SwapFlavor::LinuxAppImage,
    ];

    /// The command that puts the new build in place, per flavor.
    fn swap_token(flavor: SwapFlavor) -> &'static str {
        match flavor {
            SwapFlavor::WindowsExe => "copy /y",
            SwapFlavor::MacOsBundle => "rm -rf",
            SwapFlavor::LinuxAppImage => "mv -f",
        }
    }

    /// The command that brings the app back, per flavor.
    fn relaunch_token(flavor: SwapFlavor) -> &'static str {
        match flavor {
            SwapFlavor::WindowsExe => "start ",
            SwapFlavor::MacOsBundle => "open ",
            SwapFlavor::LinuxAppImage => "nohup ",
        }
    }

    /// Only the executable lines matter for the content assertions. The scripts
    /// carry `#` comments explaining why the rejected form is wrong, and a naive
    /// substring search over the whole script matches that prose, which is
    /// exactly what happened the first time these tests ran.
    fn commands_only(script: &str) -> String {
        script
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_flavor_waits_for_the_old_process_before_touching_anything() {
        for flavor in ALL {
            let s = render_script_for(&req(), flavor);
            let pid_at = s.find("4242").expect("the pid must appear");
            let swap_at = s
                .find(swap_token(flavor))
                .unwrap_or_else(|| panic!("{flavor:?} has no swap step"));
            assert!(
                pid_at < swap_at,
                "{flavor:?}: the wait on the pid must come BEFORE the swap, or \
                 the helper replaces a running application"
            );
        }
    }

    #[test]
    fn every_flavor_relaunches_after_swapping() {
        for flavor in ALL {
            let s = render_script_for(&req(), flavor);
            let swap_at = s
                .find(swap_token(flavor))
                .unwrap_or_else(|| panic!("{flavor:?} has no swap step"));
            let launch_at = s
                .find(relaunch_token(flavor))
                .unwrap_or_else(|| panic!("{flavor:?} has no relaunch step"));
            assert!(
                swap_at < launch_at,
                "{flavor:?}: relaunch must follow the swap"
            );
        }
    }

    /// The regression this whole split exists for.
    ///
    /// `open` is macOS. While the renderer forked on `cfg!(windows)` alone,
    /// Linux inherited the macOS branch and emitted `open`, so a Linux update
    /// swapped the AppImage correctly and then never brought the app back, and
    /// `script_relaunches_after_swapping` PASSED on Linux while doing it,
    /// because it looked for `open` on every non-Windows platform. A test that
    /// certifies the bug is worse than no test, so this one names the offending
    /// token directly.
    #[test]
    fn the_linux_script_never_uses_the_macos_open() {
        let cmds = commands_only(&render_script_for(&req(), SwapFlavor::LinuxAppImage));
        for line in cmds.lines() {
            assert!(
                !line.starts_with("open "),
                "the AppImage helper must execute the binary directly; `open` is \
                 macOS-only and leaves the user with no running app: {line}"
            );
        }
        assert!(
            cmds.contains("nohup "),
            "the relaunch must be detached, or it dies with the helper shell"
        );
    }

    #[test]
    fn every_flavor_bounds_its_wait_loop() {
        for flavor in ALL {
            let s = render_script_for(&req(), flavor);
            match flavor {
                SwapFlavor::WindowsExe => {
                    assert!(s.contains("goto wait"), "windows uses a goto loop")
                }
                // An unbounded `while kill -0` leaves a stray shell forever if
                // the app never exits, holding the old bundle's handles open.
                _ => assert!(
                    s.contains("-gt 120") && s.contains("exit 1"),
                    "{flavor:?}: the unix wait loop must give up rather than \
                     spin forever"
                ),
            }
        }
    }

    #[test]
    fn a_macos_bundle_is_removed_not_copied_over() {
        let cmds = commands_only(&render_script_for(&req(), SwapFlavor::MacOsBundle));
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

    /// The mirror image of the rule above, and the reason the two flavors cannot
    /// share a branch.
    #[test]
    fn an_appimage_is_replaced_in_one_step_and_never_deleted_first() {
        let cmds = commands_only(&render_script_for(&req(), SwapFlavor::LinuxAppImage));
        assert!(
            cmds.contains("mv -f "),
            "an AppImage is one FILE, so a single mv replaces it"
        );
        assert!(
            !cmds.contains("rm -rf") && !cmds.contains("rm -f"),
            "deleting first opens a window where `set -e` can abort with the old \
             build already gone and no new one in place; mv alone leaves the \
             installed app untouched when it fails"
        );
        assert!(
            cmds.contains("chmod +x"),
            "the executable bit is the one thing an AppImage cannot launch without"
        );
    }

    #[test]
    fn unix_flavors_have_a_shebang() {
        for flavor in [SwapFlavor::MacOsBundle, SwapFlavor::LinuxAppImage] {
            let s = render_script_for(&req(), flavor);
            assert!(
                s.starts_with("#!/bin/sh"),
                "{flavor:?} needs a shebang to be runnable"
            );
        }
    }

    #[test]
    fn paths_are_quoted_so_spaces_survive() {
        // "/Applications/StreamNook.app" is fine, but a user-relocated bundle, a
        // Linux ~/My Apps/ directory or a Windows "Program Files" path is not.
        // Unquoted, the swap silently targets the wrong path.
        for flavor in ALL {
            let mut r = req();
            r.current = PathBuf::from("/Users/a b/Applications/Stream Nook.app");
            r.replacement = PathBuf::from("/tmp/x y/Stream Nook.app");
            r.relaunch = r.current.clone();
            let s = render_script_for(&r, flavor);
            assert!(
                s.contains("\"/Users/a b/Applications/Stream Nook.app\""),
                "{flavor:?} left the target path unquoted"
            );
            assert!(
                s.contains("\"/tmp/x y/Stream Nook.app\""),
                "{flavor:?} left the replacement path unquoted"
            );
        }
    }

    /// The only test here that RUNS the script instead of reading it.
    ///
    /// Every assertion above checks that the rendered text contains or omits
    /// certain tokens, and the test this suite replaced shows exactly how far
    /// that gets you: it asserted the presence of `open` on every non-Windows
    /// platform and was green on Linux while the updater could not relaunch
    /// anything. Substring checks encode what we believe the script should say.
    /// This one encodes what it has to DO.
    ///
    /// Hermetic: its own temp directory, its own throwaway process, and it
    /// cleans up the process it relaunches. It spawns a shell and waits on a
    /// real pid, so it is slower (~2s) and more failure-prone than its
    /// neighbours; that is the price of testing the artifact rather than a
    /// description of it, and the bug it guards against shipped twice.
    ///
    /// Renders `LinuxAppImage` EXPLICITLY rather than `current_flavor()`, so it
    /// exercises the Linux script from macOS too. Every command it uses (`mv`,
    /// `chmod`, `nohup`, `kill`) is POSIX.
    #[cfg(unix)]
    #[test]
    fn the_appimage_script_really_swaps_and_relaunches() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join(format!("sn_swap_harness_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("staged")).expect("temp dir");

        let installed = dir.join("StreamNook.AppImage");
        let staged = dir.join("staged").join("StreamNook.AppImage");
        let marker = dir.join("relaunched");

        // The "old build": sleeps so it is alive while the helper waits on it.
        std::fs::write(&installed, "#!/bin/sh\nsleep 30\n").unwrap();
        // The "new build": announces itself, so the marker proves BOTH that the
        // swap put this content in place and that the relaunch executed it.
        std::fs::write(
            &staged,
            format!("#!/bin/sh\ntouch \"{}\"\nsleep 5\n", marker.display()),
        )
        .unwrap();
        super::super::fs::make_executable(&installed).unwrap();
        super::super::fs::make_executable(&staged).unwrap();

        let mut old = Command::new("/bin/sh")
            .arg(&installed)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the old build");

        let script = render_script_for(
            &SwapRequest {
                current: installed.clone(),
                replacement: staged.clone(),
                pid: old.id(),
                relaunch: installed.clone(),
            },
            SwapFlavor::LinuxAppImage,
        );
        let script_path = dir.join("helper.sh");
        std::fs::write(&script_path, &script).unwrap();
        super::super::fs::make_executable(&script_path).unwrap();

        let mut helper = Command::new("/bin/sh")
            .arg(&script_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the helper");

        // The helper is now in its wait loop. Releasing the pid is what lets it
        // proceed, which is the same sequence the real updater uses (the app
        // exits, the helper takes over).
        let _ = old.kill();
        let _ = old.wait();

        // The loop polls once a second, so this is generous rather than tight.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !marker.exists() {
            std::thread::sleep(Duration::from_millis(100));
        }

        let swapped = std::fs::read_to_string(&installed).unwrap_or_default();
        let relaunched = marker.exists();

        let _ = helper.kill();
        let _ = helper.wait();
        // The relaunched "app" is detached by design, so it is not ours to
        // wait on; it exits on its own within 5s. Remove the tree either way.
        std::thread::sleep(Duration::from_millis(200));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            swapped.contains("touch "),
            "the installed file still holds the OLD build after the swap: {swapped:?}"
        );
        assert!(
            relaunched,
            "the helper swapped the file but never relaunched it. This is the \
             exact shape of the `open` bug: the user's app disappears and the \
             new build sits on disk unrun"
        );
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
    fn helper_extension_matches_the_flavor_shell() {
        assert_eq!(script_extension_for(SwapFlavor::WindowsExe), "bat");
        assert_eq!(script_extension_for(SwapFlavor::MacOsBundle), "sh");
        assert_eq!(script_extension_for(SwapFlavor::LinuxAppImage), "sh");
        // And the running build picks the one its own shell can execute.
        assert_eq!(
            script_extension_for(current_flavor()),
            if cfg!(windows) { "bat" } else { "sh" }
        );
    }

    /// Linux must not silently ride the macOS branch again, which is what the
    /// old `cfg!(windows)`-else fork did.
    #[test]
    fn current_flavor_is_the_one_this_build_actually_needs() {
        let expected = if cfg!(windows) {
            SwapFlavor::WindowsExe
        } else if cfg!(target_os = "macos") {
            SwapFlavor::MacOsBundle
        } else {
            SwapFlavor::LinuxAppImage
        };
        assert_eq!(current_flavor(), expected);
    }
}
