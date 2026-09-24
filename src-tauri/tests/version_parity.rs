//! HARD RULE: `Cargo.toml` and `tauri.conf.json` must carry the SAME version.
//!
//! Two different values are read at runtime and both are reported as "the app
//! version": `env!("CARGO_PKG_VERSION")` (Cargo.toml, compiled into the binary)
//! and `app.package_info().version` (the merged Tauri config). Until this test
//! existed, nothing stopped them drifting, and a drift is invisible on the
//! machine that causes it:
//!
//! - `.github/workflows/build-release.yml` reads the version from Cargo.toml
//!   for the tag, the release and the update manifest, and the `push` trigger
//!   watches Cargo.toml alone. So a release whose `tauri.conf.json` was left
//!   behind still ships and tags CORRECTLY, while every client reports the old
//!   number to the admin dashboard forever: the fleet looks one version behind
//!   and no error is raised anywhere.
//! - The reverse (tauri.conf.json bumped, Cargo.toml not) fires no release at
//!   all, while a local build over-reports.
//!
//! `scripts/increment-version.js` and `scripts/release_manager.ps1` both bump
//! all three files (Cargo.toml, package.json, tauri.conf.json), so the normal
//! path is safe. This test exists for the abnormal ones: a hand-edit, a merge
//! conflict resolved toward one file, or the PowerShell regex in the release
//! script failing to match. Those are exactly the cases nobody notices.
//!
//! `tauri.android.conf.json` is deliberately NOT checked: the Android app
//! versions independently (0.1.x against the desktop 8.x), and that override
//! feeds Gradle without ever reaching Cargo. Keeping the two number spaces
//! apart is the point, so asserting they match would be wrong.
//!
//! Text parsing, like `acl_parity.rs` next door, so the test needs no runtime.

const CARGO_TOML: &str = include_str!("../Cargo.toml");
const TAURI_CONF: &str = include_str!("../tauri.conf.json");

/// The first `version = "..."` at the start of a line, which in a manifest with
/// a `[package]` section first is the package version and not a dependency's.
fn cargo_version(src: &str) -> Option<&str> {
    src.lines()
        .map(str::trim_end)
        .find_map(|line| {
            let rest = line.strip_prefix("version")?;
            let rest = rest.trim_start().strip_prefix('=')?;
            let rest = rest.trim_start().strip_prefix('"')?;
            rest.split('"').next()
        })
}

/// The top-level `"version": "..."` from the Tauri config. Read as text rather
/// than parsed, so this test pulls in no JSON dependency; the config has one
/// such key and the release scripts rewrite it in place.
fn conf_version(src: &str) -> Option<&str> {
    src.lines().map(str::trim).find_map(|line| {
        let rest = line.strip_prefix("\"version\"")?;
        let rest = rest.trim_start().strip_prefix(':')?;
        let rest = rest.trim_start().strip_prefix('"')?;
        rest.split('"').next()
    })
}

#[test]
fn cargo_and_tauri_config_versions_match() {
    let cargo = cargo_version(CARGO_TOML)
        .expect("src-tauri/Cargo.toml has no `version = \"...\"` line; did the layout change?");
    let conf = conf_version(TAURI_CONF)
        .expect("src-tauri/tauri.conf.json has no `\"version\": \"...\"` key; did it move?");

    assert_eq!(
        cargo, conf,
        "\n\nVersion drift: src-tauri/Cargo.toml says {cargo}, src-tauri/tauri.conf.json says \
         {conf}.\n\nThese are BOTH reported as the app version at runtime, and CI takes the \
         release version from Cargo.toml only, so a release would ship as v{cargo} while every \
         client told the dashboard {conf}. Bump both (scripts/increment-version.js does all \
         three files, package.json included).\n"
    );
}

/// The parsers must not silently return `None` and be `expect`ed away by a
/// future refactor, so pin them against known-shaped input.
#[test]
fn version_parsers_read_the_expected_shapes() {
    assert_eq!(
        cargo_version("[package]\nname = \"StreamNook\"\nversion = \"1.2.3\"\n"),
        Some("1.2.3")
    );
    // A dependency's inline version must not win over the package's.
    assert_eq!(
        cargo_version("[package]\nversion = \"1.2.3\"\n\n[dependencies]\nserde = { version = \"1\" }\n"),
        Some("1.2.3")
    );
    assert_eq!(cargo_version("[package]\nname = \"x\"\n"), None);

    assert_eq!(
        conf_version("{\n  \"productName\": \"StreamNook\",\n  \"version\": \"4.5.6\",\n}"),
        Some("4.5.6")
    );
    assert_eq!(conf_version("{\n  \"identifier\": \"x\"\n}"), None);
}
