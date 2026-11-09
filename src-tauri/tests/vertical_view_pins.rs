//! The vertical view pick, `video_player.vertical_view`, belongs to the core
//! (SN-40, Z4): only `change_stream_view` writes it, after the pick played,
//! and only the core reads it, to resolve a start that names no view. A shell
//! that read it would decide the view itself (and drift from the core); one
//! that wrote it could remember a picture that never played.
//!
//! Reads the sources as text, like `acl_parity.rs`. The TS type declaration
//! is the one place the name may appear.

use std::path::{Path, PathBuf};

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
        if rel == declaration {
            continue;
        }
        let text = std::fs::read_to_string(file).unwrap_or_default();
        if text.contains("vertical_view") {
            readers.push(rel.display().to_string());
        }
    }
    assert!(readers.is_empty(), "only the core reads and writes video_player.vertical_view: {readers:?}");
}
