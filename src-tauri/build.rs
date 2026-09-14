use std::env;
use std::fs;
use std::path::Path;

fn main() {
    // Load .env file from project root to inject admin IDs at compile time
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let project_root = Path::new(&manifest_dir).parent().unwrap_or(Path::new("."));

    // Keys we want to extract from .env or environment variables
    let allowed_keys = vec![
        "TWITCH_APP_CLIENT_ID",
        "TWITCH_APP_CLIENT_SECRET",
        "TWITCH_ANDROID_CLIENT_ID",
        "TWITCH_WEB_CLIENT_ID",
        // Kick OAuth app (Authorization Code + PKCE). Read at compile time the
        // same way as the Twitch keys; the code uses option_env! so a build
        // without them still compiles (Kick connect just reports "not configured").
        "KICK_APP_CLIENT_ID",
        "KICK_APP_CLIENT_SECRET",
    ];

    // Try loading from project root .env file
    let env_path = project_root.join(".env");
    if env_path.exists() {
        if let Ok(contents) = fs::read_to_string(&env_path) {
            for line in contents.lines() {
                let line = line.trim();
                // Skip comments and empty lines
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                // Parse KEY=VALUE
                if let Some((key, value)) = line.split_once('=') {
                    let key = key.trim();
                    let value = value.trim().trim_matches('"').trim_matches('\'');

                    // Only pass through specific variables we need
                    if allowed_keys.contains(&key) {
                        println!("cargo:rustc-env={}={}", key, value);
                    }
                }
            }
        }
    }

    // Also check if env vars are already set (e.g., from CI)
    // This takes precedence over .env file
    for key in &allowed_keys {
        if let Ok(val) = env::var(key) {
            println!("cargo:rustc-env={}={}", key, val);
        }
    }

    // Tell Cargo to rerun this build script if .env changes
    println!("cargo:rerun-if-changed=../.env");
    for key in allowed_keys {
        println!("cargo:rerun-if-env-changed={}", key);
    }

    ensure_potoken_bundle(&manifest_dir);

    tauri_build::build()
}

/// Keep the PO token bundle present and in step with its source.
///
/// `youtube_potoken.rs` embeds `potoken/mint.bundle.js` with `include_str!`, so it
/// must exist BEFORE this crate compiles, and it must match `mint.mjs` or playback
/// resolves through stale logic.
///
/// The bundle is generated (esbuild) and deliberately NOT committed: ~700KB of
/// minified output that rewrites wholesale on any source edit, which buries real
/// diffs in the history and cannot be merged by hand.
///
/// Built HERE rather than trusting that `npm run build:potoken` already ran. The
/// npm prebuild hook does cover `npm run tauri build`, but a bare `cargo build`
/// does not, and that failure surfaces as an unexplained `include_str!` error
/// pointing at a file nobody deleted. Regenerating costs ~100ms and only happens
/// when the source is newer than the output.
fn ensure_potoken_bundle(manifest_dir: &str) {
    let src = Path::new(manifest_dir).join("potoken").join("mint.mjs");
    let out = Path::new(manifest_dir).join("potoken").join("mint.bundle.js");
    // Rebuild whenever the source changes, not just when the output is absent.
    println!("cargo:rerun-if-changed=potoken/mint.mjs");
    if !src.exists() {
        return;
    }

    let up_to_date = match (fs::metadata(&out), fs::metadata(&src)) {
        (Ok(o), Ok(s)) => match (o.modified(), s.modified()) {
            (Ok(om), Ok(sm)) => om >= sm,
            _ => false,
        },
        _ => false,
    };
    if up_to_date {
        return;
    }

    let root = Path::new(manifest_dir).parent().unwrap_or(Path::new(".."));
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let ran = std::process::Command::new(npm)
        .args(["run", "build:potoken"])
        .current_dir(root)
        .status();

    match ran {
        Ok(status) if status.success() => {}
        _ => {
            // A stale bundle still builds and still plays; a missing one cannot
            // compile at all, so only that case is fatal, and it says what to run.
            if !out.exists() {
                panic!(
                    "potoken/mint.bundle.js is missing and `npm run build:potoken`                      could not generate it. Run that from the project root, then build again."
                );
            }
            println!(
                "cargo:warning=could not regenerate potoken/mint.bundle.js;                  building with the existing one"
            );
        }
    }
}
