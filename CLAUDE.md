# StreamNook desktop

## Hard rules

- **A new Tauri command is THREE edits, always in the same change: the `fn` (either attribute form, `#[tauri::command]` or short `#[command]`), its `generate_handler!` entry in `src-tauri/src/lib.rs` (the one registry desktop and Android share; `main.rs` is a thin binary that calls the library), and its name in `src-tauri/permissions/app-commands.toml`.** This app defines an ACL manifest, so a command that reaches the handler but not the allowlist is silently DENIED at invoke time for every window: no error surfaces, and it looks wired up in every place a human would check. That exact miss shipped real bugs three times (Viewers Also Watch rendered empty for a whole release, stage 1 of the chat-freeze recovery never ran, the open-logs-folder button did nothing). Enforced by `src-tauri/tests/acl_parity.rs` (and its vitest twin `src/mobile/acl_parity.test.ts`); `cargo test` and `npm test` fail on drift. Commands invoked from REMOTE origins additionally need `remote-bridge.toml`.
- **New settings must actually persist.** `Settings`-typed structs never reach the serde catch-all: a field the Rust struct does not name is silently dropped on save. Add the field on BOTH sides (types/index.ts and models/settings.rs) in one change.
- **Two identity-key spaces exist and must never mix in one comparison or Set.** Bare-Twitch `streamKey` (persisted legacy data), composite-always `makeKey`, and the runtime chat-slice space. Read `Brain/references/StreamNook_Identity_Keying.md` before touching any channel/slot/slice key; when you migrate a key space, grep the WRITERS, not just the readers.

## Verification gates

- Frontend: `npx tsc --noEmit`, `npx eslint <touched files>`, `npm test` (vitest), `npm run build`.
- Rust: `cargo check` / `cargo test --no-default-features` in src-tauri. While the dev app is running, the exe is locked: `cargo test` may fail to relink if Rust sources changed; stop the app first or defer.
- Never run dev servers via raw shell; never `git stash` or repo-wide destructive git ops; keep the index empty between commit groups; no Claude co-author trailers.

## Android

- This same repo builds the Android app. Android work happens from the WSL checkout of this tree (`/root/StreamNook-Android`, remote `desktop-local` = `/mnt/c/StreamNook`); Windows never runs the Android toolchain.
- **A command registered only for the mobile shell (under `#[cfg(mobile)]` / `#[cfg(target_os = "android")]` in `lib.rs`) goes in `src-tauri/permissions/mobile-commands.toml`, not `app-commands.toml`.** A name must appear in exactly one of the two files; `capabilities/mobile.json` grants both, `capabilities/desktop.json` grants only `app-commands`. Same silent-DENY failure mode as the desktop rule; both parity tests enforce it.
- Rust gate (WSL): `NDK_HOME=/root/android-sdk/ndk/26.3.11579264` with `$NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin` on `PATH`, then `cargo check --manifest-path src-tauri/Cargo.toml --target aarch64-linux-android`. Host-target cargo dies on GTK in WSL, so `cargo test` stays a desktop gate; the vitest twin covers ACL parity there.
- Build (WSL, from the repo root): `bash scripts/android-build.sh` for release, `bash scripts/android-build.sh --debug` for iteration. Always through the script: it sets `RUSTFLAGS="-C strip=debuginfo"`, which the tauri CLI otherwise discards, and forgetting it produces a working but 3x-larger APK.
- Frontend gates are the desktop ones (`npx tsc --noEmit`, `npx eslint src/mobile/`, `npm test`); `src/mobile/` is the phone-only UI, guarded by `IS_MOBILE` from `src/utils/platform.ts`.
