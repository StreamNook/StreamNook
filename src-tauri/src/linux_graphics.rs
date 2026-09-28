//! Chromium's command line for the Linux build, chosen before CEF starts.
//!
//! The Linux build embeds Chromium (see `rt.rs`), and Chromium takes its
//! rendering, video-decode and sandbox choices from switches handed to the
//! browser process at initialisation. They cannot change afterwards, so
//! `configure` runs first thing in `main`, before the runtime is configured.
//!
//! What is decided:
//!
//! - **Video decode.** Chromium ships with hardware video decode OFF on Linux;
//!   the `AcceleratedVideoDecodeLinuxGL` feature (its older name
//!   `VaapiVideoDecodeLinuxGL` is kept for coverage, unknown names are
//!   ignored) turns VA-API decode on under X11 and XWayland. Whether it
//!   engages depends on the host's VA-API driver (`intel-media-driver`,
//!   `mesa-va-drivers`, `nvidia-vaapi-driver`), which the app cannot bundle.
//! - **GPU rasterization** on every host, and on NVIDIA's proprietary driver
//!   `--ignore-gpu-blocklist`, since Chromium's blocklist keeps that driver on
//!   software paths that make video crawl.
//! - **The sandbox.** Chromium's renderers run in a sandbox built either
//!   from the setuid `chrome-sandbox` helper next to the executable or from
//!   unprivileged user namespaces. When neither can work (an AppImage mounts
//!   `nosuid`, so its helper is inert; Ubuntu 24.04 restricts unprivileged
//!   user namespaces through AppArmor) Chromium refuses to start rather than
//!   run unsandboxed, so the app passes `--no-sandbox` itself and logs why.
//!   A media app that does not start is worse than an unsandboxed renderer.
//! - **Housekeeping.** `--no-first-run` (or Chromium exits with code 28 on a
//!   fresh profile), a `basic` password store so cookies never depend on a
//!   keyring prompt, and an autoplay policy that lets a stream start without
//!   a click, the same as the other desktops.
//!
//! A user keeps the last word through `SN_CHROMIUM_ARGS`: whitespace-separated
//! switches (`--no-sandbox --enable-features=X`) appended after ours, and
//! Chromium keeps the last value of a repeated switch. `SN_CDP_PORT` opens the
//! remote debugging port, as it does for WebView2 on Windows.
//!
//! The decision is the pure `plan` so it is tested on every platform; only
//! `configure`, which reads the host, is Linux-only.

/// Why the sandbox cannot be used on this host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoSandboxReason {
    /// `user.max_user_namespaces` is 0, or the kernel forbids unprivileged
    /// user namespaces outright.
    UserNamespacesUnavailable,
    /// AppArmor restricts unprivileged user namespaces
    /// (`kernel.apparmor_restrict_unprivileged_userns`), the Ubuntu 24.04
    /// default.
    UserNamespacesRestricted,
}

impl NoSandboxReason {
    pub fn as_str(self) -> &'static str {
        match self {
            NoSandboxReason::UserNamespacesUnavailable => {
                "unprivileged user namespaces are unavailable on this kernel"
            }
            NoSandboxReason::UserNamespacesRestricted => {
                "AppArmor restricts unprivileged user namespaces (kernel.apparmor_restrict_unprivileged_userns=1)"
            }
        }
    }
}

/// The facts about the host the decision depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Host {
    /// The session is Wayland; Chromium runs through XWayland there.
    pub wayland: bool,
    /// NVIDIA's proprietary kernel driver is loaded.
    pub nvidia: bool,
    /// Why unprivileged user namespaces cannot back the sandbox, if they cannot.
    pub userns_blocked: Option<NoSandboxReason>,
    /// A setuid-root `chrome-sandbox` helper the loader will honour (owned by
    /// root, mode 4755, not on a `nosuid` mount), or `CHROME_DEVEL_SANDBOX`
    /// pointing at one.
    pub setuid_helper: bool,
}

/// How the sandbox is going to be provided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Chromium's setuid helper.
    SetuidHelper,
    /// Unprivileged user namespaces.
    UserNamespaces,
    /// Neither works; `--no-sandbox` is passed.
    Off(NoSandboxReason),
}

/// One Chromium switch: `(name, value)`. A switch without a value carries
/// its leading `--`, which is what the runtime needs to tell a switch from a
/// positional argument.
pub type Switch = (String, Option<String>);

/// What `plan` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub switches: Vec<Switch>,
    pub sandbox: Sandbox,
    /// How many switches came from `SN_CHROMIUM_ARGS`.
    pub user_switches: usize,
}

fn flag(name: &str) -> Switch {
    (format!("--{name}"), None)
}

fn valued(name: &str, value: &str) -> Switch {
    (name.to_string(), Some(value.to_string()))
}

/// Chromium features that turn hardware video decode on under X11.
const VIDEO_DECODE_FEATURES: &str = "AcceleratedVideoDecodeLinuxGL,VaapiVideoDecodeLinuxGL";

/// Decide the switches. `user_args` is the raw `SN_CHROMIUM_ARGS`, `cdp_port`
/// the parsed `SN_CDP_PORT`.
pub fn plan(host: Host, user_args: Option<&str>, cdp_port: Option<u16>) -> Plan {
    let mut switches = vec![
        flag("no-first-run"),
        valued("password-store", "basic"),
        valued("autoplay-policy", "no-user-gesture-required"),
        flag("enable-gpu-rasterization"),
        valued("enable-features", VIDEO_DECODE_FEATURES),
    ];
    if host.nvidia {
        switches.push(flag("ignore-gpu-blocklist"));
    }

    let sandbox = if host.setuid_helper {
        Sandbox::SetuidHelper
    } else {
        match host.userns_blocked {
            None => Sandbox::UserNamespaces,
            Some(reason) => Sandbox::Off(reason),
        }
    };
    if let Sandbox::Off(_) = sandbox {
        switches.push(flag("no-sandbox"));
    }

    if let Some(port) = cdp_port {
        switches.push(valued("remote-debugging-port", &port.to_string()));
    }

    let user = parse_user_args(user_args.unwrap_or(""));
    let user_switches = user.len();
    switches.extend(user);

    Plan {
        switches,
        sandbox,
        user_switches,
    }
}

/// `SN_CHROMIUM_ARGS`, whitespace-separated. `--name=value` becomes a valued
/// switch, `--name` and bare `name` a flag; Chromium takes the last value of
/// a switch given twice, so these override ours.
pub fn parse_user_args(raw: &str) -> Vec<Switch> {
    raw.split_whitespace()
        .map(|arg| {
            let arg = arg.trim_start_matches('-');
            match arg.split_once('=') {
                Some((name, value)) => valued(name, value),
                None => flag(arg),
            }
        })
        .filter(|(name, _)| name != "--")
        .collect()
}

/// Whether GTK/Chromium will run in a Wayland session, given `WAYLAND_DISPLAY`
/// and `GDK_BACKEND`. A user who sets `GDK_BACKEND=x11` is on XWayland even
/// inside a Wayland session. Informational only: the CEF runtime is X11-only.
pub fn uses_wayland(wayland_display: Option<&str>, gdk_backend: Option<&str>) -> bool {
    let has_display = wayland_display.is_some_and(|d| !d.trim().is_empty());
    let backend_first = gdk_backend
        .and_then(|b| b.split(',').next())
        .map(|b| b.trim().to_ascii_lowercase());
    match backend_first.as_deref() {
        Some("x11") => false,
        _ => has_display,
    }
}

/// Read the host and decide the switches. Returns the plan and one line for
/// the log, which cannot be written yet (this runs before logging exists).
#[cfg(target_os = "linux")]
pub fn configure() -> (Plan, String) {
    use std::path::Path;

    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let gdk_backend = std::env::var("GDK_BACKEND").ok();
    let host = Host {
        wayland: uses_wayland(wayland_display.as_deref(), gdk_backend.as_deref()),
        nvidia: Path::new("/sys/module/nvidia_drm").exists()
            || Path::new("/proc/driver/nvidia/version").exists(),
        userns_blocked: host::userns_blocked(),
        setuid_helper: host::setuid_helper_usable(),
    };
    let user_args = std::env::var("SN_CHROMIUM_ARGS").ok();
    let cdp_port: Option<u16> = std::env::var("SN_CDP_PORT")
        .ok()
        .and_then(|p| p.trim().parse::<u16>().ok())
        .filter(|p| *p != 0);

    let decided = plan(host, user_args.as_deref(), cdp_port);
    let switches = decided
        .switches
        .iter()
        .map(|(name, value)| match value {
            Some(v) => format!("{name}={v}"),
            None => name.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let sandbox = match decided.sandbox {
        Sandbox::SetuidHelper => "setuid helper".to_string(),
        Sandbox::UserNamespaces => "user namespaces".to_string(),
        Sandbox::Off(reason) => format!("OFF ({})", reason.as_str()),
    };
    let report = format!(
        "[LinuxGraphics] session={} gpu={} sandbox={sandbox} user switches={} chromium: {switches}",
        if host.wayland { "wayland (on XWayland)" } else { "x11" },
        if host.nvidia { "nvidia" } else { "other" },
        decided.user_switches,
    );
    (decided, report)
}

#[cfg(target_os = "linux")]
mod host {
    use super::NoSandboxReason;
    use std::path::Path;

    fn sysctl(path: &str) -> Option<u64> {
        std::fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    pub fn userns_blocked() -> Option<NoSandboxReason> {
        if sysctl("/proc/sys/user/max_user_namespaces") == Some(0)
            || sysctl("/proc/sys/kernel/unprivileged_userns_clone") == Some(0)
        {
            return Some(NoSandboxReason::UserNamespacesUnavailable);
        }
        if sysctl("/proc/sys/kernel/apparmor_restrict_unprivileged_userns") == Some(1) {
            return Some(NoSandboxReason::UserNamespacesRestricted);
        }
        None
    }

    /// A helper Chromium will accept: root-owned, setuid, and not inside an
    /// AppImage (mounted `nosuid`, which makes the bit inert). Chromium also
    /// honours `CHROME_DEVEL_SANDBOX`, for users who installed one themselves.
    pub fn setuid_helper_usable() -> bool {
        use std::os::unix::fs::MetadataExt;

        let candidate = std::env::var_os("CHROME_DEVEL_SANDBOX")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(|d| d.join("chrome-sandbox")))
            });
        let Some(path) = candidate else {
            return false;
        };
        if std::env::var_os("APPIMAGE").is_some() && path.starts_with(appdir()) {
            return false;
        }
        match std::fs::metadata(&path) {
            Ok(meta) => meta.uid() == 0 && meta.mode() & 0o4000 != 0,
            Err(_) => false,
        }
    }

    fn appdir() -> std::path::PathBuf {
        std::env::var_os("APPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new("/").to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(p: &Plan) -> Vec<&str> {
        p.switches.iter().map(|(n, _)| n.as_str()).collect()
    }

    fn value<'a>(p: &'a Plan, name: &str) -> Option<&'a str> {
        p.switches
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    #[test]
    fn every_host_gets_video_decode_and_gpu_rasterization() {
        let p = plan(Host::default(), None, None);
        assert!(names(&p).contains(&"--enable-gpu-rasterization"));
        assert_eq!(value(&p, "enable-features"), Some(VIDEO_DECODE_FEATURES));
        assert!(names(&p).contains(&"--no-first-run"));
        assert_eq!(value(&p, "password-store"), Some("basic"));
        assert_eq!(value(&p, "autoplay-policy"), Some("no-user-gesture-required"));
    }

    #[test]
    fn only_nvidia_ignores_the_gpu_blocklist() {
        assert!(!names(&plan(Host::default(), None, None)).contains(&"--ignore-gpu-blocklist"));
        let nv = Host { nvidia: true, ..Host::default() };
        assert!(names(&plan(nv, None, None)).contains(&"--ignore-gpu-blocklist"));
    }

    #[test]
    fn the_sandbox_stays_on_when_user_namespaces_work() {
        let p = plan(Host::default(), None, None);
        assert_eq!(p.sandbox, Sandbox::UserNamespaces);
        assert!(!names(&p).contains(&"--no-sandbox"));
    }

    #[test]
    fn a_setuid_helper_wins_over_restricted_namespaces() {
        let host = Host {
            userns_blocked: Some(NoSandboxReason::UserNamespacesRestricted),
            setuid_helper: true,
            ..Host::default()
        };
        let p = plan(host, None, None);
        assert_eq!(p.sandbox, Sandbox::SetuidHelper);
        assert!(!names(&p).contains(&"--no-sandbox"));
    }

    #[test]
    fn no_sandbox_is_passed_only_when_nothing_can_provide_one() {
        for reason in [
            NoSandboxReason::UserNamespacesRestricted,
            NoSandboxReason::UserNamespacesUnavailable,
        ] {
            let host = Host { userns_blocked: Some(reason), ..Host::default() };
            let p = plan(host, None, None);
            assert_eq!(p.sandbox, Sandbox::Off(reason));
            assert!(names(&p).contains(&"--no-sandbox"), "{reason:?}");
        }
    }

    #[test]
    fn the_debug_port_becomes_a_switch() {
        let p = plan(Host::default(), None, Some(9222));
        assert_eq!(value(&p, "remote-debugging-port"), Some("9222"));
        assert_eq!(value(&plan(Host::default(), None, None), "remote-debugging-port"), None);
    }

    #[test]
    fn user_switches_come_last_and_keep_their_shape() {
        let p = plan(Host::default(), Some("  --no-sandbox --enable-features=Foo,Bar disable-gpu "), None);
        assert_eq!(p.user_switches, 3);
        let tail = &p.switches[p.switches.len() - 3..];
        assert_eq!(
            tail,
            &[
                flag("no-sandbox"),
                valued("enable-features", "Foo,Bar"),
                flag("disable-gpu"),
            ]
        );
        assert!(parse_user_args("").is_empty());
        assert!(parse_user_args("--").is_empty());
    }

    #[test]
    fn valueless_switches_carry_the_double_dash() {
        // The runtime appends a value-less entry without `--` as a positional
        // argument, which Chromium silently ignores.
        for (name, value) in plan(Host { nvidia: true, ..Host::default() }, Some("x"), None).switches {
            if value.is_none() {
                assert!(name.starts_with("--"), "{name}");
            } else {
                assert!(!name.starts_with("--"), "{name}");
            }
        }
    }

    #[test]
    fn wayland_detection_follows_the_gdk_backend() {
        assert!(uses_wayland(Some("wayland-1"), None));
        assert!(uses_wayland(Some("wayland-1"), Some("wayland,x11")));
        assert!(!uses_wayland(Some("wayland-1"), Some("x11")));
        assert!(!uses_wayland(Some("wayland-1"), Some(" X11 ,wayland")));
        assert!(!uses_wayland(None, Some("wayland,x11")));
        assert!(!uses_wayland(Some("  "), None));
    }
}
