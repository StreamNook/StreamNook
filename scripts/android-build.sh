#!/usr/bin/env bash
#
# Build the Android app. Run from the repo root, in WSL.
#
#   scripts/android-build.sh            release (what ships)
#   scripts/android-build.sh --debug    debug (iteration)
#
# WHY THIS SCRIPT EXISTS
#
# `RUSTFLAGS="-C strip=debuginfo"` has to be on the command, and it is easy to
# forget because forgetting it produces a working build - just a hugely bloated
# one. Debug goes from a 106 MB .so to 462 MB, and the APK from 218 MB to
# 602 MB, which turns every install into a minutes-long push.
#
# It cannot live in `.cargo/config.toml` even though that file already sets it:
# the tauri CLI sets RUSTFLAGS itself, and cargo DISCARDS
# `target.<triple>.rustflags` from config.toml whenever that env var is present.
# The comment in that file claiming "rustflags set here still apply" is wrong.
#
# It also cannot go in `[profile.dev]` in Cargo.toml, which WOULD beat RUSTFLAGS,
# because cargo profiles are not per-target - it would strip line numbers out of
# desktop dev builds too, where they are genuinely wanted.
#
# So: a script. Release builds do not actually need the flag (the release profile
# builds without debug info anyway, verified: 0 `.debug_*` sections), but it is
# passed for both so there is one code path and no second thing to remember.
set -euo pipefail
cd "$(dirname "$0")/.."

MODE_ARGS=()
LABEL=release
if [[ "${1:-}" == "--debug" ]]; then
  MODE_ARGS=(--debug)
  LABEL=debug
fi

# A leftover `tauri android dev` wedges a build for ~30 minutes and can leave a
# stale APK in outputs. The bracket keeps pkill from matching its own cmdline -
# but note it must be a SEPARATE invocation from the build, because the pattern
# is a regex that would otherwise match "tauri android build" in this script's
# own parent process and kill the build before it starts (exit 15, no output).
pkill -f "[t]auri android" 2>/dev/null || true
sleep 1

echo "building $LABEL ..."
# TWO ABIs, deliberately.
#
# aarch64 is every real phone. x86_64 is what a standard Android emulator runs
# on a Windows or Intel host, and without it the app cannot start there at all:
# there is no matching libstreamnook_lib.so, so the native library fails to load
# before a line of our code runs. (Apple Silicon emulators are arm64 and were
# always fine, which is why this looked device-specific rather than ABI-shaped.)
#
# The cost is real: the native lib is most of the APK, so carrying both roughly
# doubles the download for everyone. Taken deliberately over shipping a second
# artifact, because the stable R2 key and the in-app updater both point at ONE
# StreamNook.apk.
RUSTFLAGS="-C strip=debuginfo" npx tauri android build --apk "${MODE_ARGS[@]}" --target aarch64 --target x86_64 2>&1 | tail -6

APK="src-tauri/gen/android/app/build/outputs/apk/universal/$LABEL/app-universal-$LABEL.apk"
[[ "$LABEL" == release ]] || APK="src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk"

echo
ls -l --time-style=+%H:%M:%S "$APK" | awk '{printf "%.1f MB  built %s\n", $5/1048576, $6}'

if [[ "$LABEL" == release ]]; then
  # The check that matters before publishing: a debug-signed APK looks entirely
  # normal until someone hijacks it with the well-known debug key.
  AS=$(ls "$ANDROID_HOME"/build-tools/*/apksigner 2>/dev/null | tail -1)
  echo "=== signature ==="
  "$AS" verify --print-certs "$APK" 2>&1 | grep -E "certificate DN|SHA-256 digest" | head -2
fi
