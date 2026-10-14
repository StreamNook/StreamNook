//! Every event name the core emits is listed in `tests/event_names.txt`, and
//! every name listed there is emitted.
//!
//! The iPhone app receives each event by name through `HostSink::event`, and
//! the desktop and Android frontends `listen` by name, so a renamed or new
//! event that no shell knows about fails silently. This keeps one reviewed
//! list a shell can be checked against, and makes any change to the set show
//! up in review as a change to that file.
//!
//! Like `acl_parity.rs` it reads sources as text. It finds every `.emit`,
//! `.emit_str`, `.emit_filter` (first argument) and `.emit_to`, `.emit_str_to`
//! (second argument) in `src/`, and resolves a string literal or a `&str`
//! constant to its name. A name passed through a variable cannot be resolved;
//! each such site is listed in `DYNAMIC` with the names it can carry, and a
//! new one fails the test until it is listed there too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const REGISTRY: &str = include_str!("event_names.txt");

/// Emit sites whose name arrives in a variable: (file under `src/`, the
/// argument as written, every name it can hold).
const DYNAMIC: &[(&str, &str, &[&str])] = &[
    (
        "services/channel_points_websocket_service.rs",
        "emit_event",
        &["poll-created", "poll-updated", "poll-completed"],
    ),
    ("services/slash_commands.rs", "event", &["poll-created", "poll-updated", "poll-completed"]),
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("readable source dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The source with every line comment blanked, so commented-out calls and
/// doc examples do not count.
fn without_line_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| if line.trim_start().starts_with("//") { "" } else { line })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `const NAME: &str = "value";` (any visibility), by the file that defines it:
/// names repeat across modules (a dozen services each have an `EVENT`), so a
/// constant resolves in its own module, never by name alone.
fn str_constants(sources: &[(String, String)]) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut all = BTreeMap::new();
    for (rel, source) in sources {
        let out: &mut BTreeMap<String, String> = all.entry(rel.clone()).or_default();
        for line in source.lines() {
            let line = line.trim();
            let Some(at) = line.find("const ") else { continue };
            let rest = &line[at + 6..];
            let Some((name, rest)) = rest.split_once(':') else { continue };
            let name = name.trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                continue;
            }
            let rest = rest.trim_start();
            if !(rest.starts_with("&str") || rest.starts_with("&'static str")) {
                continue;
            }
            let Some((_, value)) = rest.split_once('=') else { continue };
            let value = value.trim().trim_end_matches(';').trim();
            if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                out.insert(name.to_string(), value[1..value.len() - 1].to_string());
            }
        }
    }
    all
}

/// The value of the constant an emit names: `NAME` in the emitting file,
/// `module::NAME` in a file of that module (`use path as alias` followed), or
/// a name only one file defines. `source` is the emitting file.
fn resolve_constant<'a>(
    constants: &'a BTreeMap<String, BTreeMap<String, String>>,
    rel: &str,
    source: &str,
    arg: &str,
) -> Option<&'a String> {
    let mut path: Vec<&str> = arg.split("::").collect();
    let name = path.pop()?;
    if let Some(module) = path.last() {
        let alias = format!(" as {module};");
        let module = source
            .lines()
            .find(|l| l.trim_start().starts_with("use ") && l.trim_end().ends_with(&alias))
            .and_then(|l| l.trim_end().strip_suffix(&alias)?.rsplit("::").next())
            .unwrap_or(module);
        let file = format!("{module}.rs");
        let dir = format!("{module}/mod.rs");
        return constants
            .iter()
            .filter(|(r, _)| *r == &file || r.ends_with(&format!("/{file}")) || *r == &dir || r.ends_with(&format!("/{dir}")))
            .find_map(|(_, c)| c.get(name));
    }
    if let Some(own) = constants.get(rel).and_then(|c| c.get(name)) {
        return Some(own);
    }
    let mut found = constants.values().filter_map(|c| c.get(name));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// The first two top-level arguments of the call whose `(` ends at `start`.
fn call_args(source: &str, start: usize) -> Vec<String> {
    let bytes = source.as_bytes();
    let (mut depth, mut in_str, mut i) = (0i32, false, start);
    let mut current = String::new();
    let mut args = Vec::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            current.push(c);
            if c == '\\' && i + 1 < bytes.len() {
                current.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => {
                    in_str = true;
                    current.push(c);
                }
                '(' | '[' | '{' => {
                    depth += 1;
                    current.push(c);
                }
                ')' | ']' | '}' if depth == 0 => {
                    args.push(current.trim().to_string());
                    return args;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    args.push(current.trim().to_string());
                    current.clear();
                    if args.len() == 2 {
                        return args;
                    }
                }
                _ => current.push(c),
            }
        }
        i += 1;
    }
    args
}

/// Every emitted event name, and each emit site whose name could not be
/// resolved and is not in `DYNAMIC`.
fn emitted() -> (BTreeSet<String>, Vec<String>) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_files(&src, &mut paths);
    let sources: Vec<(String, String)> = paths
        .iter()
        .map(|p| {
            let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            (rel, without_line_comments(&std::fs::read_to_string(p).unwrap()))
        })
        .collect();
    let constants = str_constants(&sources);

    let mut names = BTreeSet::new();
    let mut unresolved = Vec::new();
    for (rel, source) in &sources {
        let mut from = 0;
        while let Some(found) = source[from..].find(".emit") {
            let at = from + found + 1;
            from = at;
            let tail = &source[at..];
            let method: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let index = match method.as_str() {
                "emit" | "emit_str" | "emit_filter" => 0,
                "emit_to" | "emit_str_to" => 1,
                _ => continue,
            };
            let after = &tail[method.len()..];
            let after_turbofish = match after.strip_prefix("::<") {
                Some(rest) => match rest.find('>') {
                    Some(end) => &rest[end + 1..],
                    None => continue,
                },
                None => after,
            };
            let trimmed = after_turbofish.trim_start();
            if !trimmed.starts_with('(') {
                continue;
            }
            let open = source.len() - trimmed.len() + 1;
            let args = call_args(source, open);
            let Some(arg) = args.get(index) else { continue };
            let arg = arg.trim_start_matches('&').trim();
            if arg.len() >= 2 && arg.starts_with('"') && arg.ends_with('"') {
                names.insert(arg[1..arg.len() - 1].to_string());
                continue;
            }
            if let Some(value) = resolve_constant(&constants, rel, source, arg) {
                names.insert(value.clone());
                continue;
            }
            match DYNAMIC.iter().find(|(file, ident, _)| file == rel && *ident == arg) {
                Some((_, _, carried)) => names.extend(carried.iter().map(|n| n.to_string())),
                None => {
                    let line = source[..at].matches('\n').count() + 1;
                    unresolved.push(format!("{rel}:{line} emits `{arg}`"));
                }
            }
        }
    }
    (names, unresolved)
}

fn registry() -> BTreeSet<String> {
    REGISTRY
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

#[test]
fn every_emitted_event_is_in_the_registry_and_back() {
    let (emitted, unresolved) = emitted();
    assert!(
        unresolved.is_empty(),
        "event names passed through a variable; list each site in DYNAMIC:\n  {}",
        unresolved.join("\n  ")
    );
    // Parser sanity: the core emits around a hundred distinct events.
    assert!(emitted.len() > 90, "only {} event names found; the scan is broken", emitted.len());

    let registry = registry();
    let missing: Vec<_> = emitted.difference(&registry).collect();
    let stale: Vec<_> = registry.difference(&emitted).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "tests/event_names.txt is out of step with the code.\n  emitted but not listed: {missing:?}\n  listed but never emitted: {stale:?}"
    );
}

#[test]
fn the_scan_resolves_literals_constants_and_targets() {
    let source = without_line_comments(
        "const NAME: &str = \"named\";\n// app.emit(\"commented\", 1);\napp.emit_to(\"main\", \"targeted\", json!({\"a\": (1, 2)}));",
    );
    assert!(!source.contains("commented"));
    let at = source.find(".emit_to(").unwrap() + ".emit_to(".len();
    assert_eq!(call_args(&source, at), ["\"main\"", "\"targeted\""]);
    let constants = str_constants(&[("x.rs".into(), source)]);
    assert_eq!(resolve_constant(&constants, "x.rs", "", "NAME").map(String::as_str), Some("named"));
    let mut two = BTreeMap::new();
    two.insert("services/a.rs".to_string(), BTreeMap::from([("EVENT".to_string(), "a-event".to_string())]));
    two.insert("commands/b.rs".to_string(), BTreeMap::from([("EVENT".to_string(), "b-event".to_string())]));
    assert_eq!(resolve_constant(&two, "services/a.rs", "", "EVENT").map(String::as_str), Some("a-event"));
    assert_eq!(resolve_constant(&two, "lib.rs", "", "crate::commands::b::EVENT").map(String::as_str), Some("b-event"));
    assert_eq!(resolve_constant(&two, "lib.rs", "use crate::commands::b as bee;", "bee::EVENT").map(String::as_str), Some("b-event"));
    assert_eq!(resolve_constant(&two, "lib.rs", "", "EVENT"), None);
}
