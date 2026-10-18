//! Every Twitch GQL operation the app sends is listed in the watchdog
//! manifest, and every entry there still exists in source.
//!
//! Twitch changes its private GQL without notice: it rotates persisted-query
//! hashes, reshapes types, and moves fields behind sign-in (`user.mods` went
//! sign-in-only and `/mods` quietly showed an empty list). StreamNook's Discord
//! bot checks every manifest operation anonymously once a day, so a break shows
//! up the same day instead of in a user report. That only works while the
//! manifest is complete, which is what this test holds.
//!
//! The manifest is private: it lives in the bot's repo, not this one. It is
//! read from `STREAMNOOK_GQL_MANIFEST`, else from
//! `../StreamNookBot/data/twitch-gql-manifest.json` beside this checkout. Where
//! neither exists (a public clone) the tests say so and pass.
//!
//! What counts as an operation, read from string literals in `src/**/*.rs`
//! outside `#[cfg(test)]` modules:
//! - a persisted-query hash: a literal of exactly 64 lowercase hex characters;
//! - an inline document: a literal that starts with `query` or `mutation` (or
//!   a bare `{ field(` selection) and contains a `{`.
//!
//! Each hash must be some entry's `hash`, and each document must contain some
//! entry's `match`, both in the same file. Another service's GraphQL (7TV,
//! YouTube) is listed under `ignore`. Entries tagged with a `tree` other than
//! `desktop` describe a build that sends GQL this tree no longer does (a phone
//! release cut from an older branch) and are not looked for here.
//!
//! The frontend and the Android Kotlin shell must not call gql.twitch.tv at
//! all: Twitch GQL lives in Rust.
//!
//! `src/twitch_gql_manifest.test.ts` is the vitest twin, for hosts where
//! `cargo test` cannot run. Keep the two in step.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const CHECK_KINDS: [&str; 4] = ["public", "signin", "mutation", "skip"];

struct Lit {
    line: usize,
    start: usize,
    raw: String,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// String literals (raw source text between the quotes, escapes left as
/// written) and a copy of the file with every literal and comment blanked, so
/// braces can be matched on code alone.
fn lex(src: &str) -> (Vec<Lit>, Vec<u8>) {
    let b = src.as_bytes();
    let mut code = b.to_vec();
    let mut lits = Vec::new();
    let blank = |code: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut code[from..to.min(b.len())] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let (mut i, mut line) = (0usize, 1usize);
    while i < b.len() {
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                blank(&mut code, start, i);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let start = i;
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
                blank(&mut code, start, i);
            }
            b'r' if matches!(b.get(i + 1), Some(b'"') | Some(b'#')) && (i == 0 || !is_ident(b[i - 1])) => {
                let mut j = i + 1;
                let mut hashes = 0;
                while b.get(j) == Some(&b'#') {
                    hashes += 1;
                    j += 1;
                }
                if b.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let start = j + 1;
                let end = src[start..].find(&close).map(|k| start + k).unwrap_or(b.len());
                lits.push(Lit { line, start, raw: src[start..end].to_string() });
                line += src[i..end].matches('\n').count();
                blank(&mut code, i, end + close.len());
                i = end + close.len();
            }
            b'\'' => {
                // A char literal (skip it, it may be '"') or a lifetime.
                if b.get(i + 1) == Some(&b'\\') {
                    let k = src[i + 2..].find('\'').map(|k| i + 2 + k).unwrap_or(b.len());
                    i = k + 1;
                } else if let Some(ch) = src[i + 1..].chars().next() {
                    let len = ch.len_utf8();
                    i += if b.get(i + 1 + len) == Some(&b'\'') { 2 + len } else { 1 };
                } else {
                    i += 1;
                }
            }
            b'"' => {
                let start = i + 1;
                let first_line = line;
                let mut j = start;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    if j < b.len() && b[j] == b'\n' {
                        line += 1;
                    }
                    j += 1;
                }
                let end = j.min(b.len());
                lits.push(Lit { line: first_line, start, raw: src[start..end].to_string() });
                blank(&mut code, i, end + 1);
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    (lits, code)
}

/// Byte ranges of `#[cfg(test)] mod name { ... }` blocks.
fn test_modules(code: &[u8]) -> Vec<(usize, usize)> {
    let text = String::from_utf8_lossy(code);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(k) = text[from..].find("#[cfg(test)]") {
        let at = from + k;
        let rest = &text[at + "#[cfg(test)]".len()..];
        let trimmed = rest.trim_start();
        let after_pub = trimmed.strip_prefix("pub ").map(str::trim_start).unwrap_or(trimmed);
        from = at + 1;
        let Some(after_mod) = after_pub.strip_prefix("mod ") else { continue };
        let Some(brace) = after_mod.find(['{', ';']) else { continue };
        if after_mod.as_bytes()[brace] != b'{' {
            continue;
        }
        let open = at + "#[cfg(test)]".len() + (rest.len() - after_mod.len()) + brace;
        let mut depth = 0usize;
        let mut end = code.len();
        for (idx, &c) in code.iter().enumerate().skip(open) {
            match c {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = idx + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((at, end));
        from = end;
    }
    out
}

fn is_hash(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn lead_trimmed(raw: &str) -> &str {
    let mut t = raw;
    loop {
        let before = t.len();
        t = t.trim_start();
        for esc in ["\\n", "\\t", "\\r"] {
            if let Some(rest) = t.strip_prefix(esc) {
                t = rest;
            }
        }
        if t.starts_with('\\') && t[1..].starts_with(char::is_whitespace) {
            t = &t[1..];
        }
        if t.len() == before {
            return t;
        }
    }
}

fn is_document(raw: &str) -> bool {
    if !raw.contains('{') {
        return false;
    }
    let t = lead_trimmed(raw);
    for kw in ["query", "mutation"] {
        if let Some(rest) = t.strip_prefix(kw) {
            return rest.starts_with(|c: char| c.is_whitespace() || c == '(' || c == '{');
        }
    }
    if !t.starts_with('{') {
        return false;
    }
    let rest = t.trim_start_matches('{').trim_start();
    if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return false;
    }
    let ident = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
    let after = rest[ident..].trim_start();
    after.starts_with('(') || after.starts_with('{')
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn files_with_ext(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "node_modules" | "build" | "generated" | ".gradle") {
                files_with_ext(&path, exts, out);
            }
        } else if exts.iter().any(|e| name.ends_with(e)) && !name.contains(".test.") {
            out.push(path);
        }
    }
}

/// The private manifest, or None (with a note) where this checkout has none.
fn manifest() -> Option<Value> {
    let path = std::env::var_os("STREAMNOOK_GQL_MANIFEST").map(PathBuf::from).unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../StreamNookBot/data/twitch-gql-manifest.json")
    });
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("gql manifest not found at {}: Twitch GQL coverage not checked", path.display());
        return None;
    };
    Some(serde_json::from_str(&text).expect("twitch-gql-manifest.json is not valid JSON"))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn on_desktop(op: &Value) -> bool {
    matches!(op.get("tree").and_then(Value::as_str), None | Some("desktop"))
}

struct Source {
    /// file -> its text
    text: BTreeMap<String, String>,
    /// (file, hash, line)
    hashes: Vec<(String, String, usize)>,
    /// (file, raw document, line)
    docs: Vec<(String, String, usize)>,
}

fn scan() -> Source {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let mut source = Source { text: BTreeMap::new(), hashes: Vec::new(), docs: Vec::new() };
    for path in files {
        let src = std::fs::read_to_string(&path).expect("read rust file").replace("\r\n", "\n");
        let rel = format!(
            "src-tauri/{}",
            path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/")
        );
        let (lits, code) = lex(&src);
        let tests = test_modules(&code);
        for lit in lits {
            if tests.iter().any(|&(a, z)| lit.start >= a && lit.start < z) {
                continue;
            }
            if is_hash(&lit.raw) {
                source.hashes.push((rel.clone(), lit.raw, lit.line));
            } else if is_document(&lit.raw) {
                source.docs.push((rel.clone(), lit.raw, lit.line));
            }
        }
        source.text.insert(rel, src);
    }
    source
}

#[test]
fn manifest_entries_are_well_formed() {
    let Some(m) = manifest() else { return };
    let clients: BTreeSet<String> = m["clientIds"]
        .as_object()
        .expect("clientIds object")
        .keys()
        .cloned()
        .collect();
    let mut ids = BTreeSet::new();
    let mut problems = Vec::new();
    for op in m["operations"].as_array().expect("operations array") {
        let id = str_of(op, "id");
        if id.is_empty() || !ids.insert(id.to_string()) {
            problems.push(format!("missing or duplicate id: {id:?}"));
        }
        if !matches!(str_of(op, "type"), "query" | "mutation") {
            problems.push(format!("{id}: type must be query or mutation"));
        }
        match str_of(op, "kind") {
            "persisted" if !is_hash(str_of(op, "hash")) => problems.push(format!("{id}: persisted entry needs a 64-hex hash")),
            "inline" if str_of(op, "match").len() < 8 => problems.push(format!("{id}: inline entry needs a match of 8+ chars")),
            "persisted" | "inline" => {}
            other => problems.push(format!("{id}: kind {other:?} is not persisted or inline")),
        }
        if !clients.contains(str_of(op, "client")) {
            problems.push(format!("{id}: client {:?} is not in clientIds", str_of(op, "client")));
        }
        for key in ["file", "feature", "auth"] {
            if str_of(op, key).is_empty() {
                problems.push(format!("{id}: missing {key}"));
            }
        }
        let checks = op.get("checks").and_then(Value::as_array).cloned().unwrap_or_default();
        if checks.is_empty() {
            problems.push(format!("{id}: needs at least one check (use expect \"skip\" with a reason)"));
        }
        for c in &checks {
            let expect = str_of(c, "expect");
            if !CHECK_KINDS.contains(&expect) {
                problems.push(format!("{id}: check expect {expect:?} is not one of {CHECK_KINDS:?}"));
            }
            if expect == "skip" && str_of(c, "reason").is_empty() {
                problems.push(format!("{id}: a skipped check needs a reason"));
            }
            if (expect == "mutation") != (str_of(op, "type") == "mutation") && expect != "skip" {
                problems.push(format!("{id}: a mutation's checks expect \"mutation\", and only a mutation's do"));
            }
            if expect != "skip" && str_of(op, "kind") == "inline" && str_of(c, "query").is_empty() {
                problems.push(format!("{id}: an inline check needs the query to send"));
            }
        }
    }
    assert!(problems.is_empty(), "twitch-gql-manifest.json:\n  {}", problems.join("\n  "));
}

#[test]
fn every_gql_operation_in_source_is_in_the_manifest() {
    let Some(m) = manifest() else { return };
    let ops: Vec<&Value> = m["operations"].as_array().unwrap().iter().filter(|o| on_desktop(o)).collect();
    let ignore = m.get("ignore").and_then(Value::as_array).cloned().unwrap_or_default();
    let source = scan();

    let mut missing = Vec::new();
    for (file, hash, line) in &source.hashes {
        let known = ops.iter().any(|o| str_of(o, "hash") == hash && str_of(o, "file") == file);
        if !known {
            missing.push(format!("{file}:{line} persisted hash {hash}"));
        }
    }
    for (file, doc, line) in &source.docs {
        let covered = ops
            .iter()
            .chain(ignore.iter().collect::<Vec<_>>().iter())
            .any(|o| {
                str_of(o, "file") == file
                    && match str_of(o, "match") {
                        "" => ignore.contains(o),
                        m => doc.contains(m),
                    }
            });
        if !covered {
            let head: String = lead_trimmed(doc).chars().take(90).collect();
            missing.push(format!("{file}:{line} inline document `{head}`"));
        }
    }
    assert!(
        missing.is_empty(),
        "Twitch GQL in source with no entry in the bot's data/twitch-gql-manifest.json \
         (add one per operation, with a check, so the daily watchdog covers it):\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn every_manifest_entry_still_exists_in_source() {
    let Some(m) = manifest() else { return };
    let source = scan();
    let mut stale = Vec::new();
    let entries = m["operations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| on_desktop(o))
        .chain(m.get("ignore").and_then(Value::as_array).map(|a| a.iter()).into_iter().flatten());
    for op in entries {
        let file = str_of(op, "file");
        let Some(text) = source.text.get(file) else {
            stale.push(format!("{}: file {file} does not exist", str_of(op, "id")));
            continue;
        };
        let hash = str_of(op, "hash");
        if !hash.is_empty() {
            if !source.hashes.iter().any(|(f, h, _)| f == file && h == hash) {
                stale.push(format!("{}: hash {hash} is no longer in {file}", str_of(op, "id")));
            }
        } else if !str_of(op, "match").is_empty() && !text.contains(str_of(op, "match")) {
            stale.push(format!("{}: match {:?} is no longer in {file}", str_of(op, "id"), str_of(op, "match")));
        }
    }
    assert!(stale.is_empty(), "stale twitch-gql-manifest.json entries:\n  {}", stale.join("\n  "));
}

/// Calls that put a request on gql.twitch.tv. A file making one must have an
/// entry, which catches a document assembled from fragments that no single
/// literal reveals.
const SENDERS: [&str; 5] = ["gql.twitch.tv", "twitch_web_gql::post", "gql_query(", "gql_public_read(", "gql_web_query("];

#[test]
fn every_file_that_sends_twitch_gql_has_an_entry() {
    let Some(m) = manifest() else { return };
    let mut known: BTreeSet<String> = BTreeSet::new();
    for op in m["operations"].as_array().unwrap().iter().filter(|o| on_desktop(o)) {
        known.insert(str_of(op, "file").to_string());
        if let Some(from) = op.get("sentFrom").and_then(Value::as_str) {
            known.insert(from.split(':').next().unwrap_or("").to_string());
        }
    }
    for key in ["ignore", "transport"] {
        for e in m.get(key).and_then(Value::as_array).into_iter().flatten() {
            known.insert(str_of(e, "file").to_string());
        }
    }
    let source = scan();
    let unknown: Vec<String> = source
        .text
        .iter()
        .filter(|(file, text)| {
            let (_, code) = lex(text);
            let cut = test_modules(&code).first().map(|&(a, _)| a).unwrap_or(text.len());
            let live = &text[..cut];
            !known.contains(file.as_str()) && SENDERS.iter().any(|s| live.contains(s))
        })
        .map(|(file, _)| file.clone())
        .collect();
    assert!(
        unknown.is_empty(),
        "these files send Twitch GQL but have no entry in twitch-gql-manifest.json \
         (add their operations, or list a pure transport file under `transport`): {unknown:?}"
    );
}

#[test]
fn the_frontend_and_kotlin_shell_never_call_twitch_gql() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    files_with_ext(&root.join("src"), &[".ts", ".tsx"], &mut files);
    files_with_ext(&root.join("src-tauri/gen/android/app/src"), &[".kt", ".java"], &mut files);
    let offenders: Vec<String> = files
        .iter()
        .filter(|p| std::fs::read_to_string(p).is_ok_and(|t| t.contains("gql.twitch.tv")))
        .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"))
        .collect();
    assert!(
        offenders.is_empty(),
        "Twitch GQL belongs in Rust (and in twitch-gql-manifest.json), not in: {offenders:?}"
    );
}

#[test]
fn the_scanner_finds_what_it_should() {
    let src = r##"
        const H: &str = "cb912a7e0789e0f8a4c85c25041a08324475831024d03d624172b59498caf085";
        fn f() { let q = "query Foo($a: ID!) { user(id: $a) { id } }"; let c = '"'; let k = "query"; }
        fn g() { let m = r#"mutation Bar($input: X!) { bar(input: $input) { ok } }"#; }
        fn h() { let t = format!("{{ users(ids: {}) {{ id }} }}", 1); let s = "\n mutation Baz { x }"; }
        #[cfg(test)]
        mod tests { fn t() { let q = "query Hidden { a { b } }"; let h = "0000000000000000000000000000000000000000000000000000000000000000"; } }
    "##;
    let (lits, code) = lex(src);
    let tests = test_modules(&code);
    let live: Vec<&Lit> = lits.iter().filter(|l| !tests.iter().any(|&(a, z)| l.start >= a && l.start < z)).collect();
    let hashes: Vec<&str> = live.iter().filter(|l| is_hash(&l.raw)).map(|l| l.raw.as_str()).collect();
    let docs: Vec<&str> = live.iter().filter(|l| is_document(&l.raw)).map(|l| lead_trimmed(&l.raw)).collect();
    assert_eq!(hashes.len(), 1);
    assert_eq!(docs.len(), 4, "{docs:?}");
    assert!(docs[0].starts_with("query Foo"));
    assert!(docs[1].starts_with("mutation Bar"));
    assert!(docs[2].starts_with("{{ users"));
    assert!(docs[3].starts_with("mutation Baz"));
}
