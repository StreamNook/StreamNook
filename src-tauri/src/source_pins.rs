//! The source-text checks in `tests/` (ACL parity, event registry, GQL manifest, window and
//! version parity, the pin tests) run inside the library's own unit-test binary.
//!
//! None of them call the library; they read source files. As integration tests they made
//! `cargo test` compile this whole crate a second time as a plain rlib just to link them,
//! roughly doubling every test build. `autotests = false` in Cargo.toml stops cargo from
//! also building them as integration tests. A new file in `tests/` must be listed here, or
//! it never runs: `every_tests_file_is_listed` fails until it is.

#[path = "../tests/acl_parity.rs"]
mod acl_parity;
#[path = "../tests/automation_hook_pins.rs"]
mod automation_hook_pins;
#[path = "../tests/core_plumbing_pins.rs"]
mod core_plumbing_pins;
#[path = "../tests/event_registry.rs"]
mod event_registry;
#[path = "../tests/gql_manifest_parity.rs"]
mod gql_manifest_parity;
#[path = "../tests/ios_invoke_parity.rs"]
mod ios_invoke_parity;
#[path = "../tests/linux_window_parity.rs"]
mod linux_window_parity;
#[path = "../tests/macos_window_parity.rs"]
mod macos_window_parity;
#[path = "../tests/p3_pins.rs"]
mod p3_pins;
#[path = "../tests/version_parity.rs"]
mod version_parity;
#[path = "../tests/vertical_view_pins.rs"]
mod vertical_view_pins;
#[path = "../tests/watch_start_pins.rs"]
mod watch_start_pins;

/// A `.rs` file dropped into `tests/` without a line above would silently never run.
#[test]
fn every_tests_file_is_listed() {
    let listed = include_str!("source_pins.rs");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut missing = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read tests/") {
        let name = entry.expect("tests/ entry").file_name().to_string_lossy().into_owned();
        if name.ends_with(".rs") && !listed.contains(&format!("#[path = \"../tests/{name}\"]")) {
            missing.push(name);
        }
    }
    assert!(
        missing.is_empty(),
        "tests/ files not listed in src/source_pins.rs (they would never run): {missing:?}"
    );
}
