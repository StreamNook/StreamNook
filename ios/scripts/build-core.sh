#!/usr/bin/env bash
# Builds the Rust core as a static library for the iPhone and generates its
# UniFFI Swift bindings. Runs in WSL from the repo root:
#
#   bash ios/scripts/build-core.sh            # release library + bindings
#   bash ios/scripts/build-core.sh --check    # type-check only, no library
#
# WSL runs every process in one systemd scope, and an OOM kill there ends the
# whole scope: every session's shells and builds, not just this one. On a busy
# VM, give the build a scope of its own with a memory cap, so running out of
# memory ends only the build:
#
#   systemd-run --unit=sn-ios-core-build -p MemoryMax=9G -p MemorySwapMax=1G \
#       --working-directory="$PWD" --setenv=HOME="$HOME" \
#       bash -lc 'bash ios/scripts/build-core.sh > /root/sn-ios-core/build.log 2>&1'
#
# Outputs (all gitignored):
#   /root/sn-ios-core/<commit>/libstreamnook_lib.a   one copy per commit built
#   ios/CoreLib/libstreamnook_lib.a                  symlink to the newest copy
#   ios/Sources/SNCore/streamnook_lib.swift          UniFFI Swift
#   ios/Sources/streamnook_libFFI/include/streamnook_libFFI.h
#
# The toolchain is the one xtool uses: swiftly's clang/llvm-ar and the iPhone
# SDK from the Darwin Swift SDK bundle. cc-rs and rustc both need SDKROOT and
# IPHONEOS_DEPLOYMENT_TARGET set, because the xcrun they would otherwise ask
# does not exist on Linux.
set -euo pipefail

# One core build at a time across every checkout: two at once overrun the VM's
# memory. The guard keeps a nested call from waiting on its own lock.
if [[ -z "${SN_BUILD_LOCKED:-}" ]]; then
    mkdir -p /root/sn-ios-core
    exec 9>/root/sn-ios-core/build.lock
    echo "Waiting for the core build lock..." >&2
    flock 9
    export SN_BUILD_LOCKED=1
fi

mode="build"
if [[ "${1:-}" == "--check" ]]; then
    mode="check"
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"

tc="$HOME/.local/share/swiftly/bin"
bundle="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"
sdks="$bundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs"
# The bundle's own ld64.lld, the one xtool links the app with. swiftly's lld
# refuses iOS and cannot read the SDK's .tbd stubs; it is only reached at all
# because some dependencies (sevenz-rust) also build a dylib.
ld="$bundle/toolset/bin/ld64.lld"
if [[ -z "${SDKROOT:-}" ]]; then
    SDKROOT="$(ls -d "$sdks"/iPhoneOS[0-9]*.sdk 2>/dev/null | sort -V | tail -1)"
fi
if [[ ! -d "$SDKROOT" ]]; then
    echo "No iPhoneOS SDK under $sdks (set SDKROOT)." >&2
    exit 1
fi
export SDKROOT
# Same floor as the Swift package.
export IPHONEOS_DEPLOYMENT_TARGET=26.0
export CC_aarch64_apple_ios="$tc/clang"
export CXX_aarch64_apple_ios="$tc/clang++"
# cc-rs defaults to `ar`, which cannot index Mach-O objects.
export AR_aarch64_apple_ios="$tc/llvm-ar"
# Only used if something links for iOS; a static library does not.
export CARGO_TARGET_AARCH64_APPLE_IOS_LINKER="$tc/clang"
# Target-scoped so host build scripts keep their own flags. strip=debuginfo
# is the same size rule scripts/android-build.sh applies.
# -fuse-ld=lld tells clang the linker is lld (so it passes -platform_version
# and -arch); --ld-path picks which lld.
export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=--ld-path=$ld -C link-arg=-isysroot -C link-arg=$SDKROOT -C strip=debuginfo"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/root/sn-ios-target}"
# The WSL VM has 16 GB shared with other builds; a full-width release build
# got rustc OOM-killed and took the VM down with it.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-6}"

echo "SDK: $SDKROOT"

# Every checkout builds into the one shared target, and cargo keys the
# StreamNook unit the same in all of them: after checkout X builds, checkout
# Y's build finds X's unit "fresh" (it re-checks X's files), skips the compile
# and hands Y X's library (or X's type-check result). So the StreamNook unit
# is forced to recompile whenever the checkout or its src-tauri tree differs
# from the last build in that profile, or the tree is dirty. Dependencies stay
# shared and compiled.
if [[ "$mode" == "check" ]]; then
    prof="$CARGO_TARGET_DIR/aarch64-apple-ios/debug"
else
    prof="$CARGO_TARGET_DIR/aarch64-apple-ios/release"
fi
stamp="$prof/.streamnook-built-from"
if [[ -n "$(git status --porcelain -- src-tauri)" ]]; then
    want="$repo dirty $(date +%s)"
else
    want="$repo $(git rev-parse HEAD:src-tauri)"
fi
if [[ "$(cat "$stamp" 2>/dev/null)" != "$want" ]]; then
    echo "Forcing the StreamNook crate to rebuild for $repo"
    rm -rf "$prof"/.fingerprint/StreamNook-* "$prof"/deps/libstreamnook_lib* "$prof"/deps/streamnook_lib* \
        "$prof"/libstreamnook_lib.*
fi
rm -f "$stamp"

if [[ "$mode" == "check" ]]; then
    cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-apple-ios --lib
    mkdir -p "$prof" && echo "$want" > "$stamp"
    exit 0
fi

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

# `--crate-type staticlib` alone: the manifest also lists cdylib, which a
# plain build would try to link for iOS.
cargo rustc --manifest-path src-tauri/Cargo.toml --release --target aarch64-apple-ios \
    --lib --crate-type staticlib -- --print native-static-libs 2>&1 | tee "$out/rustc.log"
echo "$want" > "$stamp"

commit="$(git rev-parse --short HEAD)"
if [[ -n "$(git status --porcelain -- src-tauri)" ]]; then
    commit="$commit-dirty"
fi
store="/root/sn-ios-core/$commit"
mkdir -p "$store" ios/CoreLib
cp "$CARGO_TARGET_DIR/aarch64-apple-ios/release/libstreamnook_lib.a" "$store/"
grep "native-static-libs" "$out/rustc.log" > "$store/native-static-libs.txt" || true
# What this library was really built from ("<checkout> <src-tauri tree>" or
# "<checkout> dirty <time>"). install-main.sh reuses a stored core only when
# this names the tree it wants; cores stored without it are never reused.
echo "$want" > "$store/built-from"
ln -sfn "$store/libstreamnook_lib.a" ios/CoreLib/libstreamnook_lib.a
echo "Library: $store/libstreamnook_lib.a ($(du -h "$store/libstreamnook_lib.a" | cut -f1))"

# Bindings. uniffi-bindgen reads the metadata out of the library itself and
# asks cargo about the crate, so it runs from src-tauri.
(
    cd src-tauri
    cargo run --quiet --manifest-path ../ios/tools/uniffi-bindgen/Cargo.toml -- \
        generate ../ios/CoreLib/libstreamnook_lib.a --language swift --out-dir "$out/bindings"
)
mkdir -p ios/Sources/SNCore ios/Sources/streamnook_libFFI/include
cp "$out/bindings/streamnook_lib.swift" ios/Sources/SNCore/streamnook_lib.swift
cp "$out/bindings/streamnook_libFFI.h" ios/Sources/streamnook_libFFI/include/streamnook_libFFI.h
echo "Bindings: ios/Sources/SNCore/streamnook_lib.swift, ios/Sources/streamnook_libFFI/include/streamnook_libFFI.h"
