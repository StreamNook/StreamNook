//! HARD RULE: every command registered in `generate_handler!` MUST appear in
//! `permissions/app-commands.toml`.
//!
//! The app defines an ACL manifest, which turns Tauri's authorization check ON
//! for app commands; a registered command missing from the allowlist is then
//! DENIED at invoke time for every window: silently, because most call sites
//! catch and fall back. This has shipped real user-facing bugs twice:
//! "Viewers Also Watch" rendered an empty row for a whole release
//! (get_similar_channels was never allowlisted), and stage 1 of the
//! chat-freeze recovery ladder never ran once (nudge_chat_channels).
//! reveal_main_window cost a debugging cycle the same way in dev.
//!
//! This test parses both files as text, so it needs no runtime and fails the
//! suite the moment the two drift. It deliberately derives everything from
//! `generate_handler!` and never from command attributes: commands are written
//! in BOTH attribute forms (`#[tauri::command]` and the short `#[command]`),
//! and an attribute-based audit silently under-counted by 61 once.
//! The reverse direction (allowlisted but not registered) is only a warning:
//! such an entry is inert, and it legitimately happens mid-flight when a
//! feature's ACL entry lands before its registration commit. NOTE: the test
//! harness captures output of passing tests, so the stale warning is only
//! visible via `cargo test -- --nocapture` (or on a failure).

use std::collections::BTreeSet;

const MAIN_RS: &str = include_str!("../src/main.rs");
const ACL_TOML: &str = include_str!("../permissions/app-commands.toml");

fn registered_commands() -> BTreeSet<String> {
    let start = MAIN_RS
        .find("generate_handler![")
        .expect("generate_handler! block not found in main.rs")
        + "generate_handler![".len();
    let end = MAIN_RS[start..]
        .find("])")
        .expect("generate_handler! block never closes");
    MAIN_RS[start..start + end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|tok| !tok.is_empty())
        .map(|tok| tok.rsplit("::").next().unwrap().to_string())
        .filter(|tok| {
            !tok.is_empty() && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
        .collect()
}

fn allowlisted_commands() -> BTreeSet<String> {
    let start = ACL_TOML
        .find("allow = [")
        .expect("allow array not found in app-commands.toml")
        + "allow = [".len();
    let end = ACL_TOML[start..]
        .find(']')
        .expect("allow array never closes");
    ACL_TOML[start..start + end]
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

#[test]
fn every_registered_command_is_allowlisted() {
    let registered = registered_commands();
    let allowed = allowlisted_commands();
    assert!(
        registered.len() > 100,
        "parser sanity: only {} registered commands found; the generate_handler! \
         parse has broken, fix the test before trusting it",
        registered.len()
    );

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
        "\n\nRegistered commands MISSING from permissions/app-commands.toml \
         (they are silently DENIED at invoke for every window):\n  {missing:?}\n\
         Fix: add each name to the `allow` array in the same change that \
         registers the command.\n"
    );
}
