#!/usr/bin/env bash
# Patch the AppRun hook that linuxdeploy-plugin-gtk writes into the AppDir,
# before the AppImage is repacked. Called by both Linux workflows.
#
# 1. GDK_BACKEND. The stock hook runs `export GDK_BACKEND=x11` unconditionally,
#    so on every Wayland desktop (GNOME, KDE, Hyprland) the app runs through
#    XWayland: an extra copy of every frame, fractional scaling upscaled in
#    software, and NVIDIA's slowest presentation path. Because it is a bare
#    export it also overrides a value the user set. The patched line prefers
#    native Wayland, falls back to X11 inside GTK itself, and lets the user's
#    own GDK_BACKEND win. The NVIDIA-on-Wayland renderer settings that make the
#    native path stable are applied by the app (src-tauri/src/linux_graphics.rs).
#
# 2. GIO_MODULE_DIR. The hook adds the bundled GIO modules through
#    GIO_EXTRA_MODULES but leaves the host's module directory in the search
#    path, so a newer distro's modules (libproxy, dconf, gvfs) load against the
#    older bundled GLib and fail with `undefined symbol`. Pointing
#    GIO_MODULE_DIR at the bundled directory loads only modules built for the
#    GLib we ship. The bundled directory holds libgiognutls, which WebKit needs
#    for TLS, so it must stay the one that is searched.
#
# The GTK_THEME=Adwaita export is left alone: StreamNook draws its own UI, so
# the theme reaches only native dialogs, and host themes break against the
# bundled GTK.
#
# Usage: scripts/linux-appimage-hooks.sh <path/to/StreamNook.AppDir>
set -euo pipefail

appdir="${1:?usage: $0 <AppDir>}"
hook="$appdir/apprun-hooks/linuxdeploy-plugin-gtk.sh"
test -f "$hook" || { echo "no GTK AppRun hook at $hook"; exit 1; }

want_backend='export GDK_BACKEND="${GDK_BACKEND:-wayland,x11}"'
sed -i "s|^export GDK_BACKEND=x11.*|${want_backend}|" "$hook"
grep -qxF "$want_backend" "$hook" || {
  echo "GDK_BACKEND line was not patched in $hook; the plugin's hook has changed shape"
  exit 1
}

grep -q '^export GIO_EXTRA_MODULES=' "$hook" || {
  echo "no GIO_EXTRA_MODULES export in $hook; cannot locate the bundled GIO modules"
  exit 1
}
want_gio='export GIO_MODULE_DIR="$GIO_EXTRA_MODULES"'
grep -qxF "$want_gio" "$hook" || printf '%s\n' "$want_gio" >> "$hook"

echo "patched $hook:"
grep -n 'GDK_BACKEND\|GIO_EXTRA_MODULES\|GIO_MODULE_DIR' "$hook"
