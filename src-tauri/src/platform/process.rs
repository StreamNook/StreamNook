//! Enumerate running processes.
//!
//! Reports WHAT is running. Deciding what any of it means belongs to the
//! caller: `streamer_mode` owns the list of names that imply a live broadcast,
//! and `resource_log` owns the classification of a process tree into Rust /
//! browser / GPU / renderer roles. Both are policy, and policy changes when the
//! software does, not when the OS does.
//!
//! Names come back **lowercased** on every platform so callers can compare
//! without worrying about case. They are bare names, never paths: `obs64.exe`
//! on Windows, `obs` on macOS.

use std::collections::HashSet;

/// One process in the tree.
///
/// Deliberately the same shape `resource_log::Entry` expects, so the sampler
/// can consume this directly rather than re-deriving it per platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent: u32,
    /// 0 when the platform cannot report it cheaply. Callers must treat 0 as
    /// "unknown", never as "no threads".
    pub threads: u32,
    /// Lowercased, no path, no `.exe` stripping (callers that want the suffix
    /// gone strip it themselves, because on Windows it is part of the name).
    pub name: String,
}

/// Every running process name, lowercased, without path.
///
/// Returns an empty set if enumeration fails. Callers should treat empty as
/// "could not tell", not as "nothing is running" — on the auto-detect path that
/// difference is the difference between leaving streamer mode alone and
/// switching it off underneath someone who is live.
pub fn running_names() -> HashSet<String> {
    tree().into_iter().map(|p| p.name).collect()
}

/// Every running process with its parent, for tree walking.
///
/// Empty means enumeration failed, for the same reason as above.
pub fn tree() -> Vec<ProcInfo> {
    #[cfg(windows)]
    {
        windows_tree()
    }
    #[cfg(unix)]
    {
        unix_tree()
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn windows_tree() -> Vec<ProcInfo> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return out;
        };
        let mut pe = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut ok = Process32FirstW(snap, &mut pe).is_ok();
        while ok {
            let len = pe
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(pe.szExeFile.len());
            out.push(ProcInfo {
                pid: pe.th32ProcessID,
                parent: pe.th32ParentProcessID,
                threads: pe.cntThreads,
                name: String::from_utf16_lossy(&pe.szExeFile[..len]).to_lowercase(),
            });
            ok = Process32NextW(snap, &mut pe).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// `ps` rather than a crate: this is the only place the app needs a process list
/// off Windows, and `sysinfo` would be a sizeable dependency for one
/// 30-second poll.
///
/// Two invocations are tried because `thcount` is not portable across every
/// `ps`. Losing it costs only the thread count, so the fallback is worth having
/// rather than returning nothing.
#[cfg(unix)]
fn unix_tree() -> Vec<ProcInfo> {
    fn run(args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("ps").args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    if let Some(text) = run(&["-Ao", "pid=,ppid=,thcount=,comm="]) {
        let rows = parse_ps(&text, true);
        if !rows.is_empty() {
            return rows;
        }
    }
    run(&["-Ao", "pid=,ppid=,comm="])
        .map(|t| parse_ps(&t, false))
        .unwrap_or_default()
}

/// Parse `ps` output into [`ProcInfo`].
///
/// Separated from the spawn so it can be tested on any platform, including
/// Windows CI where there is no `ps` to run.
///
/// The command name is taken as everything after the leading numeric columns
/// and then reduced to its final path component: on macOS `comm` is the
/// bundle's inner executable path (`/Applications/OBS.app/Contents/MacOS/OBS`),
/// and **a name can legitimately contain spaces** ("Streamlabs Desktop"), so it
/// cannot be split on whitespace.
#[cfg_attr(windows, allow(dead_code))]
fn parse_ps(text: &str, with_threads: bool) -> Vec<ProcInfo> {
    let numeric_cols = if with_threads { 3 } else { 2 };
    let mut out = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut nums: Vec<u32> = Vec::with_capacity(numeric_cols);
        let mut rest = line;
        let mut ok = true;
        for _ in 0..numeric_cols {
            let rest_trimmed = rest.trim_start();
            let end = rest_trimmed
                .find(char::is_whitespace)
                .unwrap_or(rest_trimmed.len());
            match rest_trimmed[..end].parse::<u32>() {
                Ok(v) => nums.push(v),
                Err(_) => {
                    ok = false;
                    break;
                }
            }
            rest = &rest_trimmed[end..];
        }
        if !ok || nums.len() < numeric_cols {
            continue;
        }
        let name_raw = rest.trim();
        if name_raw.is_empty() {
            continue;
        }
        let name = name_raw
            .rsplit('/')
            .next()
            .unwrap_or(name_raw)
            .to_lowercase();
        out.push(ProcInfo {
            pid: nums[0],
            parent: nums[1],
            threads: if with_threads { nums[2] } else { 0 },
            name,
        });
    }
    out
}

/// Every descendant of `root`, `root` itself included.
///
/// Pure over the supplied list so it is testable without touching the OS.
/// Handles the cycles a stale snapshot can contain: a pid recycled into a
/// position where it becomes its own ancestor would otherwise loop forever.
pub fn descendants_of(all: &[ProcInfo], root: u32) -> Vec<ProcInfo> {
    let mut kept: HashSet<u32> = HashSet::new();
    kept.insert(root);
    // Repeat until nothing new is added: children can appear before parents.
    loop {
        let before = kept.len();
        for p in all {
            if kept.contains(&p.parent) {
                kept.insert(p.pid);
            }
        }
        if kept.len() == before {
            break;
        }
    }
    all.iter().filter(|p| kept.contains(&p.pid)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumeration_finds_at_least_this_process() {
        let names = running_names();
        // A test binary is itself a process, so an empty set means enumeration
        // is broken rather than that the machine is idle.
        assert!(
            !names.is_empty(),
            "process enumeration returned nothing, which cannot be true while \
             this test is running"
        );
    }

    #[test]
    fn tree_reports_this_process_with_a_parent() {
        let all = tree();
        assert!(!all.is_empty(), "tree() returned nothing");
        let me = std::process::id();
        let mine = all.iter().find(|p| p.pid == me);
        assert!(
            mine.is_some(),
            "tree() should contain the running test process (pid {me})"
        );
        assert_ne!(mine.unwrap().parent, me, "a process cannot be its own parent");
    }

    #[test]
    fn parse_ps_reads_the_three_column_form() {
        let text = "    1     0    4 /sbin/launchd\n  327     1   12 /usr/libexec/logd\n";
        let rows = parse_ps(text, true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ProcInfo { pid: 1, parent: 0, threads: 4, name: "launchd".into() });
        assert_eq!(rows[1].pid, 327);
        assert_eq!(rows[1].threads, 12);
        assert_eq!(rows[1].name, "logd");
    }

    #[test]
    fn parse_ps_reads_the_two_column_fallback_with_unknown_threads() {
        let rows = parse_ps("  42   1 /usr/bin/foo\n", false);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].threads, 0, "0 means unknown, not zero threads");
        assert_eq!(rows[0].name, "foo");
    }

    #[test]
    fn process_names_containing_spaces_survive() {
        // "Streamlabs Desktop" is a real macOS process name. Splitting the line
        // on whitespace would truncate it to "streamlabs" and the broadcast
        // check would miss it.
        let rows = parse_ps("  99   1    3 /Applications/Streamlabs Desktop.app/Contents/MacOS/Streamlabs Desktop\n", true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "streamlabs desktop");
    }

    #[test]
    fn bundle_paths_reduce_to_the_bare_executable_name() {
        let rows = parse_ps("  7   1   9 /Applications/OBS.app/Contents/MacOS/OBS\n", true);
        assert_eq!(rows[0].name, "obs", "must not keep the bundle path");
    }

    #[test]
    fn parse_ps_skips_headers_and_malformed_lines() {
        let text = "  PID  PPID THCOUNT COMM\n  1 0 2 /sbin/launchd\n\n   garbage line\n";
        let rows = parse_ps(text, true);
        assert_eq!(rows.len(), 1, "only the one well-formed row should survive");
        assert_eq!(rows[0].pid, 1);
    }

    fn p(pid: u32, parent: u32, name: &str) -> ProcInfo {
        ProcInfo { pid, parent, threads: 1, name: name.into() }
    }

    #[test]
    fn descendants_collects_children_listed_before_their_parents() {
        let all = vec![
            p(300, 200, "renderer"),  // grandchild, listed first
            p(200, 100, "browser"),
            p(100, 1, "streamnook"),
            p(999, 1, "unrelated"),
        ];
        let got: HashSet<u32> = descendants_of(&all, 100).iter().map(|x| x.pid).collect();
        assert_eq!(got, HashSet::from([100, 200, 300]));
        assert!(!got.contains(&999), "an unrelated process must not be swept in");
    }

    #[test]
    fn descendants_terminates_on_a_cycle() {
        // A recycled pid can make a snapshot self-referential. Naive recursion
        // would hang; this must simply return.
        let all = vec![p(10, 11, "a"), p(11, 10, "b")];
        let got = descendants_of(&all, 10);
        assert_eq!(got.len(), 2, "both are reachable, and the walk must terminate");
    }
}
