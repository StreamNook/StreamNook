//! Whether this install has run the first-run setup itself, or carries settings
//! from another one.
//!
//! The phones back up `settings.json` (Android's backup and phone-to-phone
//! transfer, the iPhone's iCloud backup), so a fresh install can start with
//! `setup_complete` already true and no sign-in at all, which skipped the setup
//! and left a bare sign-in screen. Two marks tell the cases apart:
//!
//! - a marker file in the data dir that is never backed up (Android's backup
//!   rules name only the settings, and the iPhone shell excludes the file from
//!   iCloud backup), so it exists only on the install that wrote it;
//! - `Settings::install_marked`, which IS backed up and says the settings came
//!   from an install that had a marker.
//!
//! Settings marked but no marker here: they came from another install, so the
//! setup runs again with them as its starting choices. Neither mark: settings
//! from before this existed. The data dir then decides: anything in it beyond
//! what a backup restores means this install ran before (an update), nothing
//! means a restore from an older backup.

use std::path::Path;
use std::sync::OnceLock;

use crate::models::settings::Settings;

/// The marker's name in the data dir. Never add it to a backup rule.
pub const MARKER: &str = ".install";

/// What a backup brings back into the data dir, plus what this launch has
/// already written by the time `observe` runs.
const RESTORED_OR_NEW: &[&str] = &["settings.json", "settings-backups", "logs", MARKER];

/// Whether the data dir held anything a backup does not restore, read before
/// this launch writes anything else there.
static RAN_HERE_BEFORE: OnceLock<bool> = OnceLock::new();

/// Look at the data dir as this launch found it. Call once the data dir is
/// known and before anything else writes to it.
pub fn observe(dir: &Path) {
    let earlier = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| !RESTORED_OR_NEW.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .unwrap_or(false);
    let _ = RAN_HERE_BEFORE.set(earlier);
}

/// What `reconcile` found.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The marker is here: the settings are this install's own.
    Own,
    /// No marker yet and no setup done: a new install, now marked.
    New,
    /// An install from before the marker existed, now marked.
    Updated,
    /// The settings came from another install: the setup runs again.
    Restored,
}

/// The verdict for settings loaded with or without a marker in the data dir.
pub fn verdict(marker_present: bool, settings: &Settings, ran_here_before: bool) -> Verdict {
    if marker_present {
        Verdict::Own
    } else if !settings.setup_complete {
        Verdict::New
    } else if settings.install_marked || !ran_here_before {
        Verdict::Restored
    } else {
        Verdict::Updated
    }
}

/// Bring freshly loaded settings in line with this install, writing the
/// marker when it is missing. True when the settings changed and should be
/// written back now (so closing the app mid-setup cannot skip it next time).
pub fn reconcile(dir: &Path, settings: &mut Settings) -> bool {
    let marker = dir.join(MARKER);
    let ran_before = RAN_HERE_BEFORE.get().copied().unwrap_or(true);
    let found = verdict(marker.exists(), settings, ran_before);
    if found == Verdict::Own {
        return false;
    }
    if let Err(e) = std::fs::write(&marker, b"") {
        // Without the marker every launch would read as a restore; leave the
        // settings as they are rather than send the person through setup again.
        log::warn!("[Setup] could not write the install marker: {e}");
        return false;
    }
    // iCloud would otherwise carry it to the next iPhone, where it would vouch
    // for settings this install never saw.
    #[cfg(target_os = "ios")]
    crate::services::token_vault::exclude_from_backup(&marker);
    settings.install_marked = true;
    match found {
        Verdict::Restored => {
            log::info!("[Setup] these settings came from another install (a backup or a transfer); setup runs again");
            settings.setup_complete = false;
            true
        }
        Verdict::Updated => {
            log::info!("[Setup] marked an install from before the install marker");
            true
        }
        // A new install: saved with its first real settings write.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(setup_complete: bool, install_marked: bool) -> Settings {
        let mut s = Settings::default();
        s.setup_complete = setup_complete;
        s.install_marked = install_marked;
        s
    }

    #[test]
    fn this_installs_own_settings_are_left_alone() {
        assert_eq!(verdict(true, &settings(true, true), false), Verdict::Own);
    }

    #[test]
    fn settings_from_a_marked_install_run_the_setup_again() {
        assert_eq!(verdict(false, &settings(true, true), true), Verdict::Restored);
    }

    #[test]
    fn older_settings_with_nothing_else_here_are_a_restore() {
        assert_eq!(verdict(false, &settings(true, false), false), Verdict::Restored);
    }

    #[test]
    fn older_settings_on_an_install_that_ran_here_are_an_update() {
        assert_eq!(verdict(false, &settings(true, false), true), Verdict::Updated);
    }

    #[test]
    fn a_new_install_is_just_marked() {
        assert_eq!(verdict(false, &settings(false, false), false), Verdict::New);
    }

    #[test]
    fn a_restore_turns_setup_back_on_and_marks_the_install() {
        let dir = std::env::temp_dir().join(format!("sn-install-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = settings(true, true);
        assert!(reconcile(&dir, &mut s));
        assert!(!s.setup_complete);
        assert!(s.install_marked);
        assert!(dir.join(MARKER).exists());
        // The next launch finds the marker and leaves the settings alone.
        s.setup_complete = true;
        assert!(!reconcile(&dir, &mut s));
        assert!(s.setup_complete);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
