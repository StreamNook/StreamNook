//! Screen-region capture.
//!
//! Backs the Profile share feature, which grabs real compositor pixels rather
//! than cloning the DOM — `backdrop-filter`, paint gradients and custom fonts
//! do not survive a DOM clone.
//!
//! # Why each platform does it differently
//!
//! - **Windows**: `xcap` (static) and DXGI Output Duplication (animated), both
//!   already in `commands/screen_capture.rs`. Untouched here.
//! - **macOS**: `/usr/sbin/screencapture`, which is part of the OS. Chosen over
//!   a ScreenCaptureKit FFI bridge because it needs no Objective-C interop and
//!   no extra dependency for what is a once-per-share operation. `xcap` is not
//!   an option: the pinned 0.0.14 does not compile for macOS at all, and 0.9.x
//!   is a breaking refactor of a shipping Windows feature.
//!
//! # The permission that will bite
//!
//! macOS gates screen capture behind the **Screen Recording** TCC permission.
//! Until the user grants it, `screencapture` exits non-zero with
//! "could not create image from rect" and writes nothing. That is a *permission*
//! failure, not a bug, and [`CaptureError::PermissionDenied`] exists so the UI
//! can say so instead of reporting a generic failure.

use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum CaptureError {
    /// The requested region has zero width or height.
    EmptyRegion,
    /// The OS refused because screen-recording consent has not been granted.
    PermissionDenied,
    /// Anything else, with a loggable diagnostic.
    Failed(String),
    /// No capture backend exists for this platform.
    Unsupported,
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRegion => write!(f, "capture region has zero size"),
            Self::PermissionDenied => write!(
                f,
                "screen recording permission has not been granted to StreamNook"
            ),
            Self::Failed(e) => write!(f, "screen capture failed: {e}"),
            Self::Unsupported => write!(f, "screen capture is not supported on this platform"),
        }
    }
}

/// Build the argument list for macOS `screencapture`.
///
/// Split out from the spawn so it can be unit-tested on any platform: getting
/// the `-R` geometry string wrong is silent (you get the wrong pixels, not an
/// error), which makes it exactly the kind of thing worth pinning in a test.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn screencapture_args(x: i32, y: i32, width: u32, height: u32, out: &Path) -> Vec<String> {
    vec![
        // No shutter sound, and no cursor in the shot.
        "-x".to_string(),
        "-C".to_string(),
        "-t".to_string(),
        "png".to_string(),
        // -R takes ONE comma-joined argument. Splitting it across argv silently
        // captures the whole screen instead of the region.
        format!("-R{x},{y},{width},{height}"),
        out.to_string_lossy().to_string(),
    ]
}

/// The Linux region-capture tools we will try, in order, with their argv.
///
/// There is no single `screencapture` on Linux, so this is a preference list
/// rather than one command. Ordering is deliberate:
///
/// 1. `grim` first, because it is the only one of these that works under
///    Wayland (wlroots: Sway, Hyprland, river). The X11 tools below see a black
///    or empty screen there rather than failing cleanly, so trying them first
///    would produce a *wrong picture* instead of a fallthrough.
/// 2. `maim`, `import` (ImageMagick) and `scrot` for X11, in descending order
///    of how faithfully they handle compositing and alpha.
///
/// Each takes its geometry in a different spelling, which is exactly the kind of
/// detail that is silent when wrong: you get the wrong pixels, not an error. So
/// this is a pure function, unit-tested below, for the same reason
/// `screencapture_args` is.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_capture_commands(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    out: &Path,
) -> Vec<(&'static str, Vec<String>)> {
    let o = out.to_string_lossy().to_string();
    vec![
        // Wayland (wlroots). `-g "X,Y WxH"` is ONE argument: the space is part
        // of the geometry string, not an argv separator.
        ("grim", vec!["-g".into(), format!("{x},{y} {width}x{height}"), o.clone()]),
        // X11. maim's -g takes an X11 geometry string.
        ("maim", vec!["-g".into(), format!("{width}x{height}+{x}+{y}"), o.clone()]),
        // ImageMagick. Grabs the root window and crops; `+repage` drops the
        // crop offset from the output canvas, without which the PNG carries a
        // page geometry and decoders place the image at an offset.
        (
            "import",
            vec![
                "-window".into(),
                "root".into(),
                "-crop".into(),
                format!("{width}x{height}+{x}+{y}"),
                "+repage".into(),
                o.clone(),
            ],
        ),
        // scrot's -a is comma-separated and has no size/offset sigils at all.
        ("scrot", vec!["-o".into(), "-a".into(), format!("{x},{y},{width},{height}"), o]),
    ]
}

/// Capture a screen region and return PNG bytes.
///
/// Coordinates are screen-global and in points (the same space the caller's
/// window geometry uses).
pub fn capture_region_png(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::EmptyRegion);
    }

    #[cfg(target_os = "macos")]
    {
        let out = std::env::temp_dir().join(format!(
            "streamnook_capture_{}_{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));
        let result = std::process::Command::new("/usr/sbin/screencapture")
            .args(screencapture_args(x, y, width, height, &out))
            .output();

        let output = match result {
            Ok(o) => o,
            Err(e) => return Err(CaptureError::Failed(format!("spawn screencapture: {e}"))),
        };

        let bytes = std::fs::read(&out).ok();
        let _ = std::fs::remove_file(&out);

        match bytes {
            Some(b) if !b.is_empty() => Ok(b),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                // screencapture does not use a distinct exit code for a denied
                // TCC prompt; the message is the only signal.
                if stderr.contains("could not create image")
                    || stderr.contains("not authorized")
                    || stderr.contains("permission")
                {
                    Err(CaptureError::PermissionDenied)
                } else {
                    Err(CaptureError::Failed(format!(
                        "screencapture wrote no image (stderr: {})",
                        stderr.trim()
                    )))
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let out = std::env::temp_dir().join(format!(
            "streamnook_capture_{}_{}.png",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        ));

        let mut tried: Vec<String> = Vec::new();
        for (bin, args) in linux_capture_commands(x, y, width, height, &out) {
            let spawned = std::process::Command::new(bin).args(&args).output();
            let output = match spawned {
                // Not installed. Not an error: the next candidate is the point.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tried.push(format!("{bin} (not installed)"));
                    continue;
                }
                Err(e) => {
                    tried.push(format!("{bin} ({e})"));
                    continue;
                }
                Ok(o) => o,
            };

            let bytes = std::fs::read(&out).ok();
            let _ = std::fs::remove_file(&out);
            match bytes {
                Some(b) if !b.is_empty() => return Ok(b),
                _ => {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
                    // The portal-backed tools say so when the compositor
                    // refuses, which is the Linux analogue of macOS TCC and
                    // deserves the same distinct error rather than being
                    // reported as "the tool is broken".
                    if stderr.contains("permission")
                        || stderr.contains("denied")
                        || stderr.contains("not authorized")
                    {
                        return Err(CaptureError::PermissionDenied);
                    }
                    tried.push(format!("{bin} ({})", stderr.trim()));
                }
            }
        }

        // Naming what was tried, and what to install, because the alternative
        // is a user seeing "screen capture failed" on a machine where one
        // `apt install` fixes it.
        Err(CaptureError::Failed(format!(
            "no screen-capture tool available (tried: {}). Install `grim` on Wayland, \
             or `maim`, `imagemagick` or `scrot` on X11.",
            tried.join(", ")
        )))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Windows keeps xcap / DXGI in commands/screen_capture.rs; this module
        // is not on its path.
        let _ = (x, y, width, height);
        Err(CaptureError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_sized_regions_are_rejected_before_touching_the_os() {
        assert!(matches!(
            capture_region_png(0, 0, 0, 100),
            Err(CaptureError::EmptyRegion)
        ));
        assert!(matches!(
            capture_region_png(0, 0, 100, 0),
            Err(CaptureError::EmptyRegion)
        ));
    }

    /// Each Linux tool spells geometry differently and NONE of them error on a
    /// wrong-but-parseable one: you get the wrong pixels shared publicly. So
    /// every spelling is pinned exactly, the same way the macOS `-R` string is.
    ///
    /// **This pins CONSTRUCTION, not acceptance.** It proves the strings are the
    /// ones intended; it cannot prove a given tool build accepts them. Learned
    /// the hard way: `import -window root` is the documented ImageMagick recipe
    /// and it fails outright on the IM 7.1 build in the WSL test box, where even
    /// a bare `import -window root out.png` exits with "missing an image
    /// filename". The chain is designed to survive exactly that - a tool that
    /// errors or writes nothing falls through to the next candidate - but do not
    /// read a green test here as "capture works on Linux". Only a real desktop
    /// with `grim` (Wayland) or `maim`/`scrot` (X11) installed proves that.
    #[test]
    fn every_linux_tool_gets_its_own_geometry_spelling() {
        let cmds = linux_capture_commands(12, 34, 560, 320, Path::new("/tmp/x.png"));
        let by = |name: &str| -> Vec<String> {
            cmds.iter()
                .find(|(b, _)| *b == name)
                .unwrap_or_else(|| panic!("{name} must be a candidate"))
                .1
                .clone()
        };

        // grim: "X,Y WxH" as ONE argv entry. Split on the space it captures
        // the whole output instead.
        let grim = by("grim");
        assert_eq!(grim[0], "-g");
        assert_eq!(grim[1], "12,34 560x320");

        // maim and import share the X11 WxH+X+Y form.
        assert_eq!(by("maim")[1], "560x320+12+34");
        assert!(by("import").contains(&"560x320+12+34".to_string()));

        // scrot is comma-separated with no sigils at all.
        let scrot = by("scrot");
        assert!(scrot.contains(&"12,34,560,320".to_string()));
    }

    /// wlroots Wayland is the case where a wrong ORDER is worse than a wrong
    /// flag: the X11 tools do not fail there, they return a black frame.
    #[test]
    fn the_wayland_tool_is_tried_before_the_x11_ones() {
        let cmds = linux_capture_commands(0, 0, 10, 10, Path::new("/tmp/x.png"));
        let names: Vec<&str> = cmds.iter().map(|(b, _)| *b).collect();
        assert_eq!(
            names.first(),
            Some(&"grim"),
            "grim must come first: under Wayland the X11 tools succeed and \
             return a black image rather than failing through to the next one"
        );
        for x11 in ["maim", "import", "scrot"] {
            assert!(names.contains(&x11), "{x11} must remain a candidate");
        }
    }

    #[test]
    fn import_drops_the_crop_page_offset() {
        let import = linux_capture_commands(5, 5, 10, 10, Path::new("/tmp/x.png"))
            .into_iter()
            .find(|(b, _)| *b == "import")
            .expect("import candidate")
            .1;
        assert!(
            import.contains(&"+repage".to_string()),
            "without +repage the PNG keeps the crop offset as a page geometry \
             and decoders place the image inset instead of at the origin"
        );
    }

    #[test]
    fn linux_negative_origins_survive_every_spelling() {
        // A monitor left of the primary gives negative screen coords.
        let cmds = linux_capture_commands(-1920, -50, 100, 100, Path::new("/tmp/x.png"));
        let flat: Vec<String> = cmds.into_iter().flat_map(|(_, a)| a).collect();
        assert!(flat.iter().any(|a| a == "-1920,-50 100x100"), "grim");
        assert!(flat.iter().any(|a| a == "100x100+-1920+-50"), "maim/import");
        assert!(flat.iter().any(|a| a == "-1920,-50,100,100"), "scrot");
    }

    #[test]
    fn region_geometry_is_one_comma_joined_argument() {
        let args = screencapture_args(12, 34, 560, 320, Path::new("/tmp/x.png"));
        let region: Vec<&String> = args.iter().filter(|a| a.starts_with("-R")).collect();
        assert_eq!(region.len(), 1, "exactly one -R argument");
        assert_eq!(
            region[0], "-R12,34,560,320",
            "geometry must be joined into the -R flag; split across argv it \
             silently captures the whole screen instead of erroring"
        );
    }

    #[test]
    fn negative_origins_survive_the_format() {
        // A second display left of the primary gives negative screen coords.
        let args = screencapture_args(-1920, -50, 100, 100, Path::new("/tmp/x.png"));
        assert!(args.iter().any(|a| a == "-R-1920,-50,100,100"));
    }

    #[test]
    fn output_path_is_the_last_argument_and_png_is_requested() {
        let args = screencapture_args(0, 0, 10, 10, Path::new("/tmp/shot.png"));
        assert_eq!(args.last().map(String::as_str), Some("/tmp/shot.png"));
        let t = args.iter().position(|a| a == "-t").expect("-t present");
        assert_eq!(args.get(t + 1).map(String::as_str), Some("png"));
    }

    #[test]
    fn capture_errors_describe_themselves_usefully() {
        // The UI surfaces these strings, so a permission problem must not read
        // as a generic failure.
        assert!(CaptureError::PermissionDenied
            .to_string()
            .contains("screen recording permission"));
        assert!(CaptureError::EmptyRegion.to_string().contains("zero size"));
    }
}
