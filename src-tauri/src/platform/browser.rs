//! Open a URL in the user's default browser.
//!
//! Deliberately a free function taking only a `&str`, because the OAuth paths
//! that need it (Kick's consent leg) run before, or outside of, anywhere an
//! `AppHandle` is conveniently in scope. The `tauri-plugin-opener` route is used
//! elsewhere in the app where a handle IS available; this is the handle-free
//! equivalent, not a competing mechanism.

use anyhow::{anyhow, Result};

/// Open `url` in whatever the OS considers the default browser.
///
/// # Platform notes
///
/// - **Windows** uses `rundll32 url.dll,FileProtocolHandler` rather than
///   `cmd /C start`. `cmd` treats the `&` between OAuth query parameters as a
///   command separator and truncates the URL at the first one, which silently
///   breaks every consent link. `rundll32` takes the URL as a single argument
///   and opens it verbatim. Do not "simplify" this back to `start`.
/// - **macOS** uses `/usr/bin/open`, which has no such quoting hazard because
///   the URL arrives as one `argv` entry.
/// - **Linux** uses `xdg-open`, same reasoning.
pub fn open_in_browser(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        // NOT `cmd /C start`: cmd treats the `&` between OAuth query params as a
        // command separator and truncates the URL at the first `&`. rundll32's
        // FileProtocolHandler takes the URL as a single arg and opens it verbatim.
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|e| anyhow!("couldn't open the browser: {}", e))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .spawn()
            .map_err(|e| anyhow!("couldn't open the browser: {}", e))?;
        return Ok(());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| anyhow!("couldn't open the browser (is xdg-utils installed?): {}", e))?;
        return Ok(());
    }

    #[allow(unreachable_code)]
    {
        let _ = url;
        Err(anyhow!("opening a browser is not supported on this platform"))
    }
}
