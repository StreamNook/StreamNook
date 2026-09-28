//! Linux configuration invariants.
//!
//! HARD RULE: `tauri.linux.conf.json`'s window object must match the base
//! `tauri.conf.json` window object on every key except the intentional
//! Linux-only ones.
//!
//! Tauri merges a platform config into the base one with JSON Merge Patch
//! (RFC 7396), which replaces arrays wholesale. `app.windows` is an array, so
//! the Linux file restates the entire main window object to change one key,
//! and an omitted property silently reverts to Tauri's built-in default rather
//! than to StreamNook's value. Nothing but this test ties the two copies
//! together; `macos_window_parity.rs` guards the macOS copy the same way.
//!
//! When a difference is genuinely intended, add the key to `LINUX_ONLY_KEYS`
//! with a comment explaining why.

use serde_json::Value;

const BASE_JSON: &str = include_str!("../tauri.conf.json");
const LINUX_JSON: &str = include_str!("../tauri.linux.conf.json");

/// Keys the Linux window object is allowed to differ on, or to introduce.
const LINUX_ONLY_KEYS: &[&str] = &[
    // false on Linux: the setup hook builds the main window from this same
    // entry so it can decide transparency at runtime (see
    // src/linux_window_frame.rs). Leaving it true would create a second
    // window with the same label.
    "create",
];

fn window_object(raw: &str, which: &str) -> serde_json::Map<String, Value> {
    let root: Value =
        serde_json::from_str(raw).unwrap_or_else(|e| panic!("{which} is not valid JSON: {e}"));
    let windows = root
        .get("app")
        .and_then(|a| a.get("windows"))
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{which} has no app.windows array"));
    let main = windows
        .iter()
        .find(|w| w.get("label").and_then(Value::as_str) == Some("main"))
        .unwrap_or_else(|| panic!("{which} has no window labelled \"main\""));
    main.as_object()
        .expect("window entry is not an object")
        .clone()
}

#[test]
fn linux_window_config_matches_base_except_for_intentional_keys() {
    let base = window_object(BASE_JSON, "tauri.conf.json");
    let linux = window_object(LINUX_JSON, "tauri.linux.conf.json");

    let mut problems: Vec<String> = Vec::new();
    for (key, base_value) in &base {
        if LINUX_ONLY_KEYS.contains(&key.as_str()) {
            continue;
        }
        match linux.get(key) {
            None => problems.push(format!(
                "`{key}` is in tauri.conf.json but MISSING from tauri.linux.conf.json \
                 (RFC 7396 replaces the whole array, so Linux would fall back to \
                 Tauri's default, not to {base_value})"
            )),
            Some(v) if v != base_value => problems.push(format!(
                "`{key}` differs: base = {base_value}, Linux = {v}. If that is \
                 intended, add `{key}` to LINUX_ONLY_KEYS with a reason."
            )),
            Some(_) => {}
        }
    }
    for key in linux.keys() {
        if !base.contains_key(key) && !LINUX_ONLY_KEYS.contains(&key.as_str()) {
            problems.push(format!(
                "`{key}` exists only in tauri.linux.conf.json and is not in \
                 LINUX_ONLY_KEYS. Add it there with a reason, or remove it."
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "tauri.linux.conf.json has drifted from tauri.conf.json:\n  - {}",
        problems.join("\n  - ")
    );
}

/// The setup hook creates the Linux main window itself; if the config also
/// created it, the second build would fail on the duplicate label.
#[test]
fn linux_main_window_is_built_by_the_setup_hook_not_the_config() {
    let linux = window_object(LINUX_JSON, "tauri.linux.conf.json");
    assert_eq!(
        linux.get("create").and_then(Value::as_bool),
        Some(false),
        "tauri.linux.conf.json needs `create: false` on the main window"
    );
}
