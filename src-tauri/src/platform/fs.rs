//! Filesystem operations whose meaning is universal but whose implementation is not.

use anyhow::{Context, Result};
use std::path::Path;

/// Mark `path` executable.
///
/// # Why this exists
///
/// Windows decides executability from the file extension, so this is a no-op
/// there. Unix decides it from the mode bits, and **a zip created on Windows
/// carries no Unix mode**, so `ZipArchive::extract` lands the file at 0644 and
/// `Command::new(entry)` then fails with "Permission denied".
///
/// That is exactly how a macOS plugin install fails: the download succeeds, the
/// signature verifies, the manifest parses, the install reports success, and
/// then the plugin host cannot spawn the binary. The failure surfaces far from
/// its cause, which is why the installer calls this explicitly rather than
/// trusting whoever packaged the artifact to have zipped it on the right OS.
pub fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("cannot stat {} to make it executable", path.display()))?;
        let mut perms = meta.permissions();
        // Mirror the read bits into execute: a file readable by group/other
        // stays consistent, and one that is owner-only stays owner-only.
        let mode = perms.mode();
        let with_exec = mode | ((mode & 0o444) >> 2);
        perms.set_mode(with_exec);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("cannot chmod +x {}", path.display()))?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        // Executability is extension-derived on Windows; nothing to set. Still
        // verify the file is there, so a caller gets the same error it would on
        // unix rather than a silent success over a missing path.
        std::fs::metadata(path)
            .with_context(|| format!("cannot stat {}", path.display()))?;
        Ok(())
    }
}

/// True when `path` is executable by its owner.
///
/// Always true for an existing file on Windows, where the question is not
/// meaningful; the function exists so callers and tests can be written once.
pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o100 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("streamnook_fs_test_{name}"));
        let mut f = std::fs::File::create(&p).expect("create temp file");
        f.write_all(b"#!/bin/sh\necho hi\n").expect("write temp file");
        p
    }

    #[test]
    fn make_executable_marks_a_plain_file_runnable() {
        let p = temp_file("exec");
        // A freshly created file is 0644 on unix, i.e. NOT executable — which is
        // precisely the state a Windows-made zip leaves a plugin binary in.
        #[cfg(unix)]
        assert!(!is_executable(&p), "fixture should start non-executable");

        make_executable(&p).expect("make_executable should succeed");
        assert!(is_executable(&p), "file should be executable afterwards");

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn make_executable_is_idempotent() {
        let p = temp_file("idem");
        make_executable(&p).expect("first call");
        make_executable(&p).expect("second call should not fail");
        assert!(is_executable(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn make_executable_reports_a_missing_file_rather_than_succeeding() {
        let missing = std::env::temp_dir().join("streamnook_fs_test_definitely_absent");
        let _ = std::fs::remove_file(&missing);
        assert!(
            make_executable(&missing).is_err(),
            "a missing path must be an error on every platform, not a silent Ok"
        );
    }

    #[cfg(unix)]
    #[test]
    fn owner_only_files_stay_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let p = temp_file("owner_only");
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&p, perms).unwrap();

        make_executable(&p).unwrap();

        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o700,
            "0600 should become 0700, not world-executable (got {mode:o})"
        );
        let _ = std::fs::remove_file(&p);
    }
}
