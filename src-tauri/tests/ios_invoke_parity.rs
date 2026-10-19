//! Every command the iPhone app invokes must be one the core registers in
//! `generate_handler!` (the iPhone's `invoke` goes through the same registry).
//!
//! A name the core does not know fails at run time, and the Swift call sites
//! mostly read the answer with `try?`, so the failure is silent: the iPhone's
//! chat once asked for `get_settings`, which the core never had, and drew every
//! room with built-in defaults whatever the Chat page saved. FakeCore answered
//! the name, so every test passed.
//!
//! Parses the files as text, like `acl_parity.rs`: the Swift sources' string
//! literals passed to `invoke(...)` (and `SettingsStore.send(...)`) against the
//! names in `generate_handler!`. The fakes in `SNKit/Fake*.swift` answer
//! commands rather than call them, so they are left out.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const LIB_RS: &str = include_str!("../src/lib.rs");

fn registered_commands() -> BTreeSet<String> {
    let start = LIB_RS.find("generate_handler![").expect("generate_handler! block not found") + "generate_handler![".len();
    let end = LIB_RS[start..].find("])").expect("generate_handler! block never closes");
    LIB_RS[start..start + end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .map(|tok| tok.rsplit("::").next().unwrap_or("").to_string())
        .filter(|tok| !tok.is_empty() && tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .collect()
}

fn swift_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            swift_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "swift") {
            out.push(path);
        }
    }
}

/// `invoke("name"` and `send("name"` literals in one Swift file.
fn invoked_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for opener in ["invoke(\"", "send(\""] {
        let mut rest = text;
        while let Some(at) = rest.find(opener) {
            rest = &rest[at + opener.len()..];
            let name: String = rest.chars().take_while(|c| *c != '"').collect();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                names.push(name);
            }
        }
    }
    names
}

#[test]
fn every_command_the_iphone_invokes_is_registered() {
    let sources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ios/Sources");
    let mut files = Vec::new();
    swift_files(&sources, &mut files);
    assert!(!files.is_empty(), "no Swift sources under {}", sources.display());

    let registered = registered_commands();
    let mut missing = BTreeSet::new();
    let mut seen = 0;
    for file in files {
        if file.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Fake")) {
            continue;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for name in invoked_names(&text) {
            seen += 1;
            if !registered.contains(&name) {
                missing.insert(format!("{name} ({})", file.file_name().unwrap().to_string_lossy()));
            }
        }
    }
    assert!(seen > 0, "found no invoke calls in the Swift sources");
    assert!(missing.is_empty(), "the iPhone invokes commands the core never registers: {missing:?}");
}

#[test]
fn names_are_read_from_invoke_and_send_literals() {
    let text = r#"try await core.invoke("load_settings", "{}"); send("set_settings_fields", x); invoke(cmd, y)"#;
    assert_eq!(invoked_names(text), vec!["load_settings", "set_settings_fields"]);
}
