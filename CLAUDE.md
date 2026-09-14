# StreamNook desktop

## Hard rules

- **A new Tauri command is THREE edits, always in the same change: the `fn` (either attribute form, `#[tauri::command]` or short `#[command]`), its `generate_handler!` entry in main.rs, and its name in `src-tauri/permissions/app-commands.toml`.** This app defines an ACL manifest, so a command that reaches the handler but not the allowlist is silently DENIED at invoke time for every window: no error surfaces, and it looks wired up in every place a human would check. That exact miss shipped real bugs three times (Viewers Also Watch rendered empty for a whole release, stage 1 of the chat-freeze recovery never ran, the open-logs-folder button did nothing). Enforced by `src-tauri/tests/acl_parity.rs`; `cargo test` fails on drift. Commands invoked from REMOTE origins additionally need `remote-bridge.toml`.
- **New settings must actually persist.** `Settings`-typed structs never reach the serde catch-all: a field the Rust struct does not name is silently dropped on save. Add the field on BOTH sides (types/index.ts and models/settings.rs) in one change.
- **Two identity-key spaces exist and must never mix in one comparison or Set.** Bare-Twitch `streamKey` (persisted legacy data), composite-always `makeKey`, and the runtime chat-slice space. Read `Brain/references/StreamNook_Identity_Keying.md` before touching any channel/slot/slice key; when you migrate a key space, grep the WRITERS, not just the readers.

## Verification gates

- Frontend: `npx tsc --noEmit`, `npx eslint <touched files>`, `npm test` (vitest), `npm run build`.
- Rust: `cargo check` / `cargo test --no-default-features` in src-tauri. While the dev app is running, the exe is locked: `cargo test` may fail to relink if Rust sources changed; stop the app first or defer.
- Never run dev servers via raw shell; never `git stash` or repo-wide destructive git ops; keep the index empty between commit groups; no Claude co-author trailers.
