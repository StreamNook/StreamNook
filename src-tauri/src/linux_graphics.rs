//! WebKitGTK renderer settings for Linux, chosen before GTK starts.
//!
//! WebKitGTK reads these from the environment once, when GTK initialises and the
//! first web view is created, so they have to be in place before tao builds the
//! event loop. `configure` runs first thing in `run()` for that reason.
//!
//! NVIDIA's proprietary driver is the case that needs help. WebKitGTK's DMA-BUF
//! renderer hands the compositor buffers in formats that driver does not always
//! provide, which shows up as a blank window, a Wayland "Error 71" crash, or
//! redraws that crawl (a Tauri app on Hyprland + RTX measured a 176 ms median
//! redraw interval with the default path and 32 ms with shared-memory
//! transport). `WEBKIT_DMABUF_RENDERER_FORCE_SHM` keeps GPU rendering and only
//! changes how finished frames reach the compositor, so it fixes the transport
//! without falling back to CPU rendering. `__NV_DISABLE_EXPLICIT_SYNC` avoids the
//! driver's explicit-sync path on Wayland, the documented cause of Error 71,
//! at no measured cost.
//!
//! A variable the user has already set is never touched: exporting one of these
//! (to any value, `0` included) is how someone opts out of our choice. A user who
//! has taken over the renderer with `WEBKIT_DISABLE_DMABUF_RENDERER` or
//! `WEBKIT_DISABLE_COMPOSITING_MODE` gets no renderer setting from us at all.
//!
//! The decision is the pure `plan` so it is tested on every platform; only
//! `configure`, which reads the host and writes the environment, is Linux-only.

/// The facts about the host the decision depends on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Host {
    /// GTK will open its window through Wayland rather than X11/XWayland.
    pub wayland: bool,
    /// NVIDIA's proprietary kernel driver is loaded.
    pub nvidia: bool,
}

/// One environment variable to set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    pub value: &'static str,
}

/// What `plan` decided: the variables to set, and the ones it wanted but left
/// alone because the user already controls them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    pub apply: Vec<Setting>,
    pub kept_user_value: Vec<&'static str>,
}

const FORCE_SHM: Setting = Setting {
    name: "WEBKIT_DMABUF_RENDERER_FORCE_SHM",
    value: "1",
};
const NV_EXPLICIT_SYNC_OFF: Setting = Setting {
    name: "__NV_DISABLE_EXPLICIT_SYNC",
    value: "1",
};

/// Variables that mean the user is choosing WebKit's renderer themselves.
const USER_RENDERER_CHOICES: [&str; 2] = [
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WEBKIT_DISABLE_COMPOSITING_MODE",
];

/// Decide which variables to set. `is_set` reports whether a variable is
/// already present in the environment.
pub fn plan(host: Host, is_set: impl Fn(&str) -> bool) -> Plan {
    let mut out = Plan::default();
    if !host.nvidia {
        return out;
    }

    let mut wanted = vec![FORCE_SHM];
    if host.wayland {
        wanted.push(NV_EXPLICIT_SYNC_OFF);
    }

    let user_owns_renderer = USER_RENDERER_CHOICES.iter().any(|v| is_set(v));
    for setting in wanted {
        let renderer_setting = setting.name == FORCE_SHM.name;
        if is_set(setting.name) || (renderer_setting && user_owns_renderer) {
            out.kept_user_value.push(setting.name);
        } else {
            out.apply.push(setting);
        }
    }
    out
}

/// Whether GTK will use Wayland, given `WAYLAND_DISPLAY` and `GDK_BACKEND`.
/// The AppImage defaults `GDK_BACKEND` to `wayland,x11`; a user who sets it to
/// `x11` is on XWayland even inside a Wayland session.
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

/// Read the host, set the planned variables, and return a line for the log.
/// Must run before any other thread starts: the environment is only safe to
/// modify while the process is single-threaded.
#[cfg(target_os = "linux")]
pub fn configure() -> String {
    use std::path::Path;

    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let gdk_backend = std::env::var("GDK_BACKEND").ok();
    let host = Host {
        wayland: uses_wayland(wayland_display.as_deref(), gdk_backend.as_deref()),
        nvidia: Path::new("/sys/module/nvidia_drm").exists()
            || Path::new("/proc/driver/nvidia/version").exists(),
    };

    let decided = plan(host, |name| std::env::var_os(name).is_some());
    for s in &decided.apply {
        std::env::set_var(s.name, s.value);
    }

    let applied = if decided.apply.is_empty() {
        "none".to_string()
    } else {
        decided
            .apply
            .iter()
            .map(|s| format!("{}={}", s.name, s.value))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let kept = if decided.kept_user_value.is_empty() {
        "none".to_string()
    } else {
        decided.kept_user_value.join(", ")
    };
    format!(
        "[LinuxGraphics] session={} gpu={} GDK_BACKEND={} applied: {applied}; left to the user: {kept}",
        if host.wayland { "wayland" } else { "x11" },
        if host.nvidia { "nvidia" } else { "other" },
        gdk_backend.as_deref().unwrap_or("(unset)"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none_set(_: &str) -> bool {
        false
    }

    #[test]
    fn nothing_changes_without_nvidia() {
        for wayland in [false, true] {
            let p = plan(Host { wayland, nvidia: false }, none_set);
            assert!(p.apply.is_empty() && p.kept_user_value.is_empty());
        }
    }

    #[test]
    fn nvidia_on_x11_gets_shared_memory_transport_only() {
        let p = plan(Host { wayland: false, nvidia: true }, none_set);
        assert_eq!(p.apply, vec![FORCE_SHM]);
    }

    #[test]
    fn nvidia_on_wayland_also_turns_off_explicit_sync() {
        let p = plan(Host { wayland: true, nvidia: true }, none_set);
        assert_eq!(p.apply, vec![FORCE_SHM, NV_EXPLICIT_SYNC_OFF]);
    }

    #[test]
    fn a_value_the_user_set_is_never_overridden() {
        let p = plan(Host { wayland: true, nvidia: true }, |n| {
            n == "WEBKIT_DMABUF_RENDERER_FORCE_SHM"
        });
        assert_eq!(p.apply, vec![NV_EXPLICIT_SYNC_OFF]);
        assert_eq!(p.kept_user_value, vec!["WEBKIT_DMABUF_RENDERER_FORCE_SHM"]);
    }

    #[test]
    fn a_user_renderer_choice_suppresses_ours() {
        for choice in USER_RENDERER_CHOICES {
            let p = plan(Host { wayland: true, nvidia: true }, |n| n == choice);
            assert_eq!(p.apply, vec![NV_EXPLICIT_SYNC_OFF], "with {choice} set");
            assert_eq!(p.kept_user_value, vec![FORCE_SHM.name]);
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
