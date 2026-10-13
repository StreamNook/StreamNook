#!/usr/bin/env bash
# Builds the Rust core as a static library for the iPhone and generates its
# UniFFI Swift bindings. Runs in WSL from the repo root:
#
#   bash ios/scripts/build-core.sh            # release library + bindings
#   bash ios/scripts/build-core.sh --check    # type-check only, no library
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

mode="build"
if [[ "${1:-}" == "--check" ]]; then
    mode="check"
fi

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"

tc="$HOME/.local/share/swiftly/bin"
sdks="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/Developer/Platforms/iPhoneOS.platform/Developer/SDKs"
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
export CARGO_TARGET_AARCH64_APPLE_IOS_RUSTFLAGS="-C link-arg=-fuse-ld=lld -C link-arg=-isysroot -C link-arg=$SDKROOT -C strip=debuginfo"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/root/sn-ios-target}"

echo "SDK: $SDKROOT"

if [[ "$mode" == "check" ]]; then
    cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-apple-ios --lib
    exit 0
fi

out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

# `--crate-type staticlib` alone: the manifest also lists cdylib, which a
# plain build would try to link for iOS.
cargo rustc --manifest-path src-tauri/Cargo.toml --release --target aarch64-apple-ios \
    --lib --crate-type staticlib -- --print native-static-libs 2>&1 | tee "$out/rustc.log"

commit="$(git rev-parse --short HEAD)"
if [[ -n "$(git status --porcelain -- src-tauri)" ]]; then
    commit="$commit-dirty"
fi
store="/root/sn-ios-core/$commit"
mkdir -p "$store" ios/CoreLib
cp "$CARGO_TARGET_DIR/aarch64-apple-ios/release/libstreamnook_lib.a" "$store/"
grep "native-static-libs" "$out/rustc.log" > "$store/native-static-libs.txt" || true
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
