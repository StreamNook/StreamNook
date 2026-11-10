//! The vertical view pick, `video_player.vertical_view`, belongs to the core
//! (SN-40, Z4): only `change_stream_view` writes it, after the pick played,
//! and only the core reads it, to resolve a start that names no view. A shell
//! that read it would decide the view itself (and drift from the core); one
//! that wrote it could remember a picture that never played.
//!
//! Reads the sources as text, like `acl_parity.rs`. The TS type declaration
//! is the one place the name may appear in code; a comment may name it
//! anywhere (`is_comment`, the same rule as the Swift twin in
//! `StartArgsPinTests`).

use std::path::{Path, PathBuf};

/// A `//`, `///`, `/*` or ` * ` line, read trimmed.
fn is_comment(line: &str) -> bool {
    let code = line.trim_start();
    code.starts_with("//") || code.starts_with("/*") || code.starts_with('*')
}

fn files(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            files(&path, exts, out);
        } else if path.extension().and_then(|e| e.to_str()).is_some_and(|e| exts.contains(&e)) {
            out.push(path);
        }
    }
}

#[test]
fn no_shell_reads_or_writes_the_vertical_pick() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut sources = Vec::new();
    files(&root.join("src"), &["ts", "tsx"], &mut sources);
    assert!(sources.len() > 100, "the TS scan found only {} files", sources.len());
    // The phone tree carries the iPhone sources; the desktop repo has none.
    files(&root.join("ios/Sources"), &["swift"], &mut sources);

    let declaration = Path::new("src").join("types").join("index.ts");
    let mut readers = Vec::new();
    for file in &sources {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        // The declaration, and tests that pin the shells never write it (they
        // have to name the field to look for it).
        let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if rel == declaration || name.contains(".test.") {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap_or_default();
        for line in text.lines().filter(|l| l.contains("vertical_view") && !is_comment(l)) {
            readers.push(format!("{}: {}", rel.display(), line.trim()));
        }
    }
    assert!(readers.is_empty(), "only the core reads and writes video_player.vertical_view: {readers:?}");
}

#[test]
fn a_comment_may_name_the_pick() {
    assert!(is_comment("        /// `video_player.vertical_view`, written only by a view change."));
    assert!(is_comment("  // vertical_view"));
    assert!(is_comment("   * vertical_view, in a block comment"));
    assert!(is_comment("/* vertical_view */"));
    assert!(!is_comment("  const v = settings.video_player.vertical_view; // a reader"));
    assert!(!is_comment("let pick = s.vertical_view"));
}

/// X8: v2 is fetched only when the caller asks (a phone's solo start, a
/// phone's VOD open, a Vertical quality menu). Tiles, Cast, the ad pivot and
/// the soaks use the entry points that never ask, and those pass no ask.
#[test]
fn tiles_cast_the_pivot_and_the_soaks_never_ask_v2() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let read = |rel: &str| {
        std::fs::read_to_string(src.join(rel))
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .replace("\r\n", "\n")
    };
    let asking = ["resolve_live_with(", "resolve_live_resilient_with(", "resolve_vod_with(", "fetch_auth_masters(", "DualAsk"];
    for (rel, entry) in [
        ("commands/multi_nook.rs", "tr::resolve_live_resilient("),
        ("services/cast_session.rs", "tr::resolve_live_resilient("),
        ("services/ad_bypass/pivot.rs", "twitch_resolver::resolve_live("),
        ("services/link_soak.rs", "twitch_resolver::resolve_live("),
    ] {
        let text = read(rel);
        assert!(text.contains(entry), "{rel} resolves through {entry}");
        for ask in asking {
            assert!(!text.contains(ask), "{rel} must not ask v2 ({ask})");
        }
    }
    // The entry points that never ask pass no ask, and no ask spawns no v2.
    let resolver = read("services/twitch_resolver.rs");
    assert!(resolver.contains("resolve_live_with(channel, oauth_token, quality, None)"));
    assert!(resolver.contains("resolve_live_resilient_with(channel, oauth_token, quality, retry_delay_secs, budget_secs, None)"));
    assert!(resolver.contains("resolve_vod_with(vod_id, oauth_token, quality, None)"));
    assert!(resolver.contains("fetch_auth_masters(&channel, Some(token), ask.is_some())"));
    assert!(resolver.contains("fetch_auth_masters(&channel, oauth_token, ask.is_some())"));
    assert!(resolver.contains("fetch_vod_masters(vod_id, oauth_token, ask.is_some())"));
    let proxy = read("services/auth_proxy.rs");
    assert!(proxy.contains("fetch_auth_masters(channel, oauth_token, false)"), "the one-master fetch never asks");
    assert!(proxy.contains("want_dual.then(|| spawn_v2("), "v2 is spawned only when asked");
}
