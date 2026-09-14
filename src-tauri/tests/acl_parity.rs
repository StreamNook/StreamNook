//! HARD RULE: every command registered in `generate_handler!` MUST appear in
//! exactly one of `permissions/app-commands.toml` (desktop + shared commands)
//! and `permissions/mobile-commands.toml` (commands that exist only in the
//! mobile shell).
//!
//! The app defines an ACL manifest, which turns Tauri's authorization check ON
//! for app commands; a registered command missing from every allowlist is then
//! DENIED at invoke time for every window: silently, because most call sites
//! catch and fall back. This has shipped real user-facing bugs twice:
//! "Viewers Also Watch" rendered an empty row for a whole release
//! (get_similar_channels was never allowlisted), and stage 1 of the
//! chat-freeze recovery ladder never ran once (nudge_chat_channels).
//! reveal_main_window cost a debugging cycle the same way in dev.
//!
//! Desktop and Android share one registry: `src/lib.rs` (`main.rs` is a thin
//! binary that calls `streamnook_lib::run`). `#[cfg(...)]` guards inside the
//! block are ignored on purpose, so a command that only compiles on one
//! platform still needs an allowlist entry; a name listed in BOTH files is a
//! mistake (the mobile file exists precisely for names the desktop file does
//! not carry), so that is asserted too.
//!
//! This test parses the files as text, so it needs no runtime and fails the
//! suite the moment they drift. It deliberately derives everything from
//! `generate_handler!` and never from command attributes: commands are written
//! in BOTH attribute forms (`#[tauri::command]` and the short `#[command]`),
//! and an attribute-based audit silently under-counted by 61 once.
//! The reverse direction (allowlisted but not registered) is only a warning:
//! such an entry is inert, and it legitimately happens mid-flight when a
//! feature's ACL entry lands before its registration commit. NOTE: the test
//! harness captures output of passing tests, so the stale warning is only
//! visible via `cargo test -- --nocapture` (or on a failure).
//!
//! `src/mobile/acl_parity.test.ts` is the vitest twin of this file: it checks
//! the same invariant where `npm test` runs but `cargo test` cannot (the WSL
//! Android build host has no GTK for a host-target cargo). Keep the two in
//! step.

use std::collections::BTreeSet;

const LIB_RS: &str = include_str!("../src/lib.rs");
const APP_TOML: &str = include_str!("../permissions/app-commands.toml");
const MOBILE_TOML: &str = include_str!("../permissions/mobile-commands.toml");

fn registered_commands() -> BTreeSet<String> {
    let start = LIB_RS
        .find("generate_handler![")
        .expect("generate_handler! block not found in lib.rs")
        + "generate_handler![".len();
    let end = LIB_RS[start..]
        .find("])")
        .expect("generate_handler! block never closes");
    LIB_RS[start..start + end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.rsplit("::").next().unwrap().to_string())
        .filter(|tok| !tok.is_empty() && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect()
}

/// Names in one permission file's `allow = [...]` array.
fn allowlisted_commands(toml: &str, file: &str) -> BTreeSet<String> {
    let start = toml
        .find("allow = [")
        .unwrap_or_else(|| panic!("allow array not found in {file}"))
        + "allow = [".len();
    let end = toml[start..].find(']').expect("allow array never closes");
    toml[start..start + end]
        .lines()
        .map(|line| line.split('#').next().unwrap_or(""))
        .flat_map(|line| {
            line.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn app_commands() -> BTreeSet<String> {
    allowlisted_commands(APP_TOML, "app-commands.toml")
}

fn mobile_commands() -> BTreeSet<String> {
    allowlisted_commands(MOBILE_TOML, "mobile-commands.toml")
}

#[test]
fn no_command_is_listed_in_both_permission_files() {
    let app = app_commands();
    let mobile = mobile_commands();
    let both: Vec<&String> = app.intersection(&mobile).collect();
    assert!(
        both.is_empty(),
        "\n\nCommands listed in BOTH permissions/app-commands.toml and \
         permissions/mobile-commands.toml:\n  {both:?}\n\
         Fix: mobile-commands.toml carries only names app-commands.toml does not; \
         remove the duplicate from one file.\n"
    );
}

#[test]
fn every_registered_command_is_allowlisted() {
    let registered = registered_commands();
    let app = app_commands();
    let mobile = mobile_commands();
    assert!(
        registered.len() > 100,
        "parser sanity: only {} registered commands found; the generate_handler! \
         parse has broken, fix the test before trusting it",
        registered.len()
    );

    let allowed: BTreeSet<String> = app.union(&mobile).cloned().collect();
    let missing: Vec<&String> = registered.difference(&allowed).collect();
    let stale: Vec<&String> = allowed.difference(&registered).collect();

    if !stale.is_empty() {
        eprintln!(
            "WARNING: allowlisted but not registered (inert; remove when sure, or a \
             feature's registration is mid-flight): {stale:?}"
        );
    }

    assert!(
        missing.is_empty(),
        "\n\nRegistered commands MISSING from both permissions/app-commands.toml and \
         permissions/mobile-commands.toml (they are silently DENIED at invoke for \
         every window):\n  {missing:?}\n\
         Fix: add each name to the `allow` array of app-commands.toml (or \
         mobile-commands.toml for a command registered under a mobile-only cfg) \
         in the same change that registers the command.\n"
    );
}
