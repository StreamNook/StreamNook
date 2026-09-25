//! Installs the AppImage's own desktop entry and icons into the user's data dir.
//!
//! An AppImage is a single file: nothing it carries is visible to the desktop
//! until something installs it. On Wayland that matters more than it looks,
//! because a window cannot hand the compositor an icon. GNOME, KDE and Hyprland
//! bars find the icon, the launcher entry and the app's name by matching the
//! window's app id (`StreamNook`) to a `StreamNook.desktop` file. With none
//! installed the dock shows a generic icon, the app is missing from every
//! launcher, and `streamnook://` links have nothing to open them.
//!
//! So when the app runs from an AppImage it copies the bundled
//! `StreamNook.desktop` into `$XDG_DATA_HOME/applications` with `Exec` pointed at
//! the AppImage's real path, copies the bundled hicolor icons alongside, and
//! makes it the handler for `streamnook://`.
//!
//! The entry carries a marker line, and a `StreamNook.desktop` without it (a
//! distro package, or one the user wrote) is never overwritten. Files are
//! rewritten only when their content changed, so an unchanged AppImage costs a
//! few reads per launch. `TryExec` hides the entry if the AppImage is deleted.
//!
//! The text transforms are pure and tested on every platform; only `install`,
//! which touches the filesystem and runs `xdg-mime`, is Linux-only.

/// File name of the installed entry. Must equal the window's Wayland app id.
pub const ENTRY_FILE: &str = "StreamNook.desktop";
/// Marks an entry this module wrote, so it is safe to rewrite.
pub const MARKER: &str = "X-StreamNook-AppImage=true";
const SCHEME_MIME: &str = "x-scheme-handler/streamnook";

/// Quote a path for an `Exec`/`TryExec` value per the Desktop Entry spec:
/// double quotes around it, with `"`, `` ` ``, `$` and `\` escaped inside.
pub fn quote_exec_arg(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 2);
    out.push('"');
    for c in path.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Escape a plain string value (`TryExec`, unlike `Exec`, is never quoted: GLib
/// looks the value up as a literal path, so quotes would make it name a file
/// that does not exist and GLib would drop the whole entry).
pub fn escape_string_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for (i, c) in value.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            ' ' if i == 0 => out.push_str("\\s"),
            _ => out.push(c),
        }
    }
    out
}

/// Turn the bundled entry into the installed one: `Exec` and `TryExec` point at
/// the AppImage, `%u` passes `streamnook://` links through, and the marker is
/// added. Every other key the bundle declares is kept as is.
pub fn render_entry(bundled: &str, appimage: &str) -> String {
    let exec = format!("{} %u", quote_exec_arg(appimage));
    let try_exec = escape_string_value(appimage);
    let mut out = String::new();
    let mut in_main = false;
    let mut wrote_ours = false;
    for line in bundled.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_main && !wrote_ours {
                push_ours(&mut out, &exec, &try_exec);
                wrote_ours = true;
            }
            in_main = trimmed == "[Desktop Entry]";
        }
        if in_main
            && (trimmed.starts_with("Exec=")
                || trimmed.starts_with("TryExec=")
                || trimmed == MARKER)
        {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !wrote_ours {
        push_ours(&mut out, &exec, &try_exec);
    }
    out
}

fn push_ours(out: &mut String, exec: &str, try_exec: &str) {
    out.push_str(&format!("Exec={exec}\n"));
    out.push_str(&format!("TryExec={try_exec}\n"));
    out.push_str(MARKER);
    out.push('\n');
}

/// Whether an existing file at the entry's path may be replaced: only when
/// there is none, or it is one this module wrote.
pub fn may_overwrite(existing: Option<&str>) -> bool {
    match existing {
        None => true,
        Some(text) => text.lines().any(|l| l.trim() == MARKER),
    }
}

/// Install or refresh the entry and icons. Returns a line for the log.
/// Does nothing unless the app is running from an AppImage.
#[cfg(target_os = "linux")]
pub fn install() -> String {
    match install_inner() {
        Ok(outcome) => format!("[DesktopEntry] {outcome}"),
        Err(e) => format!("[DesktopEntry] not installed: {e}"),
    }
}

#[cfg(target_os = "linux")]
fn install_inner() -> Result<String, String> {
    use std::fs;
    use std::path::{Path, PathBuf};

    let appimage = match std::env::var("APPIMAGE") {
        Ok(p) if Path::new(&p).is_absolute() && Path::new(&p).is_file() => p,
        _ => return Ok("skipped: not running from an AppImage".into()),
    };
    let appdir = PathBuf::from(std::env::var("APPDIR").map_err(|_| "APPDIR is not set")?);

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("neither XDG_DATA_HOME nor HOME is set")?;

    let bundled = fs::read_to_string(appdir.join(ENTRY_FILE))
        .map_err(|e| format!("bundled {ENTRY_FILE} unreadable: {e}"))?;
    let apps_dir = data_home.join("applications");
    let target = apps_dir.join(ENTRY_FILE);
    let existing = fs::read_to_string(&target).ok();
    if !may_overwrite(existing.as_deref()) {
        return Ok(format!(
            "left alone: {} exists and was not written by the AppImage",
            target.display()
        ));
    }

    // Icons first, so the entry never points at an icon that is not there yet.
    let mut icons_written = 0usize;
    let icon_root = appdir.join("usr/share/icons/hicolor");
    if let Ok(sizes) = fs::read_dir(&icon_root) {
        for size in sizes.flatten() {
            let src = size.path().join("apps/StreamNook.png");
            let Ok(bytes) = fs::read(&src) else { continue };
            let dest_dir = data_home
                .join("icons/hicolor")
                .join(size.file_name())
                .join("apps");
            let dest = dest_dir.join("StreamNook.png");
            if fs::read(&dest).ok().as_deref() == Some(bytes.as_slice()) {
                continue;
            }
            fs::create_dir_all(&dest_dir).map_err(|e| format!("{}: {e}", dest_dir.display()))?;
            fs::write(&dest, &bytes).map_err(|e| format!("{}: {e}", dest.display()))?;
            icons_written += 1;
        }
    }

    let rendered = render_entry(&bundled, &appimage);
    if existing.as_deref() == Some(rendered.as_str()) {
        return Ok(format!("unchanged ({icons_written} icon(s) refreshed)"));
    }
    fs::create_dir_all(&apps_dir).map_err(|e| format!("{}: {e}", apps_dir.display()))?;
    fs::write(&target, &rendered).map_err(|e| format!("{}: {e}", target.display()))?;

    // Both tools are optional: without them the icon and launcher entry still
    // work, only the streamnook:// default is not set.
    let db = std::process::Command::new("update-desktop-database")
        .arg(&apps_dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let mime = std::process::Command::new("xdg-mime")
        .args(["default", ENTRY_FILE, SCHEME_MIME])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    Ok(format!(
        "{} {} for {appimage} ({icons_written} icon(s) written; desktop database {}; streamnook:// handler {})",
        if existing.is_some() { "updated" } else { "installed" },
        target.display(),
        if db { "refreshed" } else { "not refreshed" },
        if mime { "set" } else { "not set" },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = "[Desktop Entry]\nCategories=AudioVideo;Video;\nExec=StreamNook\nStartupWMClass=StreamNook\nIcon=StreamNook\nName=StreamNook\nType=Application\nMimeType=x-scheme-handler/streamnook\n";

    #[test]
    fn exec_points_at_the_appimage_and_passes_links() {
        let out = render_entry(BUNDLED, "/home/me/Apps/StreamNook.AppImage");
        assert!(out.contains("Exec=\"/home/me/Apps/StreamNook.AppImage\" %u\n"));
        // TryExec is a literal path: quoting it makes GLib reject the entry.
        assert!(out.contains("TryExec=/home/me/Apps/StreamNook.AppImage\n"));
        assert!(!out.contains("Exec=StreamNook\n"));
        assert!(out.contains(MARKER));
    }

    #[test]
    fn every_other_key_is_kept() {
        let out = render_entry(BUNDLED, "/a.AppImage");
        for key in [
            "Icon=StreamNook",
            "StartupWMClass=StreamNook",
            "MimeType=x-scheme-handler/streamnook",
            "Name=StreamNook",
            "Categories=AudioVideo;Video;",
        ] {
            assert!(out.contains(key), "{key} missing from:\n{out}");
        }
    }

    #[test]
    fn rendering_is_stable_so_unchanged_launches_write_nothing() {
        let once = render_entry(BUNDLED, "/a.AppImage");
        assert_eq!(render_entry(&once, "/a.AppImage"), once);
        assert_eq!(once.matches("Exec=").count(), 2, "Exec and TryExec only");
    }

    #[test]
    fn a_moved_appimage_rewrites_exec() {
        let old = render_entry(BUNDLED, "/old/StreamNook.AppImage");
        let new = render_entry(&old, "/new/StreamNook.AppImage");
        assert!(new.contains("/new/StreamNook.AppImage"));
        assert!(!new.contains("/old/StreamNook.AppImage"));
    }

    #[test]
    fn paths_are_quoted_per_the_desktop_entry_spec() {
        assert_eq!(quote_exec_arg("/a b/c"), "\"/a b/c\"");
        assert_eq!(quote_exec_arg("/a\"$`\\b"), "\"/a\\\"\\$\\`\\\\b\"");
    }

    #[test]
    fn try_exec_is_never_quoted_even_with_spaces() {
        let out = render_entry(BUNDLED, "/home/me/My Apps/StreamNook.AppImage");
        assert!(out.contains("TryExec=/home/me/My Apps/StreamNook.AppImage\n"));
        assert!(out.contains("Exec=\"/home/me/My Apps/StreamNook.AppImage\" %u\n"));
        assert_eq!(escape_string_value(" a\\b"), "\\sa\\\\b");
    }

    #[test]
    fn only_our_own_entry_is_ever_overwritten() {
        assert!(may_overwrite(None));
        assert!(may_overwrite(Some(&render_entry(BUNDLED, "/a.AppImage"))));
        assert!(!may_overwrite(Some(BUNDLED)));
    }

    #[test]
    fn other_sections_keep_their_own_exec() {
        let bundled = format!("{BUNDLED}\n[Desktop Action new]\nName=New\nExec=StreamNook --new\n");
        let out = render_entry(&bundled, "/a.AppImage");
        assert!(out.contains("[Desktop Action new]\nName=New\nExec=StreamNook --new\n"));
        let main = out.split("[Desktop Action").next().unwrap();
        assert!(main.contains("Exec=\"/a.AppImage\" %u") && main.contains(MARKER));
    }
}
