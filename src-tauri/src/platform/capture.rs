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

    #[cfg(not(target_os = "macos"))]
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
