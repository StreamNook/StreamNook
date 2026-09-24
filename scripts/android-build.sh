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

  # Write the update manifest FROM THE BUILD, so it cannot disagree with the APK.
  #
  # It used to be hand-written, and on 2026-09-19 the published manifest served
  # 0.1.11 while tauri.android.conf.json said 0.1.12 - so no phone was ever
  # offered 0.1.12, and the live data agreed: 13 members on 0.1.11 and zero on
  # 0.1.12. Nothing in increment-version.js, release_manager.ps1 or this script
  # touched the Android version or its manifest, so the only link between the
  # build and what phones were told was somebody retyping it.
  #
  # Version comes from the same config Gradle builds from; sha256 and size come
  # from the APK that was just produced. Notes are carried over from whatever is
  # currently published, because CHANGELOG.md is the desktop changelog and the
  # Android notes are written per release by hand.
  #
  # The UPLOAD is still Brandon's: it needs the R2 credentials and is the step
  # that makes a release live. This only removes the transcription.
  VERSION=$(node -e "process.stdout.write(require('./src-tauri/tauri.android.conf.json').version)")
  SHA=$(sha256sum "$APK" | cut -d' ' -f1)
  SIZE=$(stat -c%s "$APK")
  NOTES=$(curl -fsS --max-time 20 "https://streamnook.app/api/v1/update-android" 2>/dev/null \
    | node -e "let s='';process.stdin.on('data',d=>s+=d).on('end',()=>{try{process.stdout.write(JSON.parse(s).notes||'')}catch{process.stdout.write('')}})" \
    || echo "")
  OUT=latest-android.json
  VERSION="$VERSION" SHA="$SHA" SIZE="$SIZE" NOTES="$NOTES" node -e '
    const fs = require("fs");
    fs.writeFileSync("latest-android.json", JSON.stringify({
      version: process.env.VERSION,
      download_url: "https://streamnook.app/download/android",
      bundle_name: "StreamNook.apk",
      sha256: process.env.SHA,
      size: Number(process.env.SIZE),
      notes: process.env.NOTES,
    }, null, 2));
  '
  echo
  echo "=== $OUT (v$VERSION) ==="
  echo "Wrote $OUT from this build. Notes were carried over from the live manifest;"
  echo "edit them before publishing if this release needs its own."
  echo "To publish (yours to run, after the APK is uploaded):"
  echo "  wrangler r2 object put streamnook-downloads/$OUT --file=$OUT --content-type=application/json --remote"
fi
