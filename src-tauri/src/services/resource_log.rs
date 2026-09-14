//! Process-tree resource telemetry: one `[Resource]` line per minute in the
//! file log, so a memory report can be read off the log instead of needing a
//! live debugger on the user's machine.
//!
//! What a user sees in Task Manager is not one process: it is StreamNook.exe
//! (Rust), the WebView2 browser process, its GPU process, one renderer per
//! window, a few utilities, and any plugin child processes. A leak in any of
//! them reads as "StreamNook is at 800 MB". This walks the whole tree from
//! our own pid (Toolhelp32 snapshot), reads working set and private bytes per
//! process (`K32GetProcessMemoryInfo`), handle and thread counts, and asks
//! WebView2 itself which pid is which kind (`ICoreWebView2Environment8::
//! GetProcessInfos`) so no command lines or PEB reads are involved.
//!
//! Why Rust: the tree outlives any window (main is destroyed on close-to-tray
//! and recreated on demand), process enumeration is a Win32 job, and the
//! sample must never cost the renderer anything. Cost: one snapshot plus one
//! `OpenProcess` per member every 60 s (microseconds); every 5 minutes with
//! no window open. No state is kept between ticks.
//!
//! Line shape (MB, working set / private; h = handles, t = threads):
//! `[Resource] tree ws=1019 priv=790 | rust 203/163 h617 t31 | browser
//! 154/39 h1457 t49 | gpu 218/269 h822 t73 | renderer#17196 387/299 h465 t32
//! | utility[2] 74/29 | plugin ad-bypass 16/6 | plugin drops-farmer 16/6`

use std::collections::HashMap;
use std::time::Duration;

use log::{debug, info};
use tauri::{AppHandle, Manager};

/// Cadence while at least one window exists.
const ACTIVE_PERIOD: Duration = Duration::from_secs(60);
/// Cadence with no window at all (living in the tray).
const TRAY_PERIOD: Duration = Duration::from_secs(300);
/// First sample after boot, so a fresh session has an early baseline.
const FIRST_SAMPLE_DELAY: Duration = Duration::from_secs(20);
/// How long one tick waits for the UI thread to answer the WebView2
/// process-kind query. A wedged UI thread (the ui_hang_watchdog case) must
/// not stall telemetry; the tick logs the tree unclassified instead.
const KIND_QUERY_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// StreamNook.exe itself.
    Rust,
    /// WebView2 browser (the environment's parent process).
    Browser,
    /// WebView2 GPU process (decode surfaces, compositor).
    Gpu,
    /// WebView2 renderer: one per window.
    Renderer,
    /// WebView2 utility processes (network, audio, storage).
    Utility,
    /// A WebView2 process WebView2 did not classify, or the classification
    /// query did not answer (crashpad, sandbox helper, a wedged UI thread).
    WebViewOther,
    /// A direct child that is not WebView2: a plugin-host process.
    Plugin,
    /// Anything deeper in the tree that is not WebView2 (a plugin's conhost).
    Other,
}

/// One process in the tree, as sampled.
#[derive(Debug, Clone)]
pub struct ProcRow {
    pub pid: u32,
    /// Exe name without the `.exe` suffix.
    pub name: String,
    pub kind: Kind,
    pub ws_mb: u64,
    pub private_mb: u64,
    pub handles: u32,
    pub threads: u32,
}

/// A process as the snapshot reports it, before memory is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub pid: u32,
    pub parent: u32,
    pub threads: u32,
    pub name: String,
}

/// Start the sampler. Idempotent per process in practice (called once from
/// the setup hook); a second call would only double the log lines.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_SAMPLE_DELAY).await;
        loop {
            let has_window = !app.webview_windows().is_empty();
            let app_for_tick = app.clone();
            match tokio::task::spawn_blocking(move || tick(&app_for_tick)).await {
                Ok(Some(line)) => info!("[Resource] {line}"),
                Ok(None) => debug!("[Resource] sample unavailable this tick"),
                Err(e) => debug!("[Resource] sampler task failed: {e}"),
            }
            tokio::time::sleep(if has_window { ACTIVE_PERIOD } else { TRAY_PERIOD }).await;
        }
    });
}

fn tick(app: &AppHandle) -> Option<String> {
    let kinds = imp::webview_kinds(app);
    let rows = imp::sample_tree(&kinds)?;
    Some(format!("{} | {}", format_line(&rows), cache_summary(app)))
}

/// Entry counts of the bounded Rust caches, so a rising `rust` figure has a
/// suspect list next to it. Every read is a try-lock (a busy cache prints
/// `?` this tick rather than making the sampler wait on the chat path).
fn cache_summary(app: &AppHandle) -> String {
    fn n(v: Option<usize>) -> String {
        v.map(|x| x.to_string()).unwrap_or_else(|| "?".to_string())
    }
    fn pair(v: Option<(usize, usize)>) -> String {
        v.map(|(a, b)| format!("{a}/{b}")).unwrap_or_else(|| "?".to_string())
    }
    let mut parts: Vec<String> = Vec::with_capacity(16);
    for (name, count) in crate::services::irc_service::cache_counts() {
        parts.push(format!("{name}={}", n(count)));
    }
    let emote_sets = app
        .try_state::<crate::commands::emotes::EmoteServiceState>()
        .and_then(|st| st.0.try_read().ok().and_then(|svc| svc.cache_len()));
    parts.push(format!("emote_sets={}", n(emote_sets)));
    parts.push(format!(
        "profiles={}",
        n(crate::services::profile_cache_service::PROFILE_CACHE.cache_len())
    ));
    parts.push(format!(
        "badges_ch/users={}",
        pair(crate::commands::badge_service::cache_counts())
    ));
    parts.push(format!(
        "history_users/msgs={}",
        pair(crate::services::user_message_history_service::UserMessageHistoryService::global().cache_counts())
    ));
    parts.push(format!(
        "manifest={}",
        n(crate::services::universal_cache_service::manifest_len())
    ));
    parts.push(format!("emoji={}", n(crate::commands::app::emoji_cache_len())));
    parts.push(format!("bttv={}", n(crate::services::bttv_pro_service::cache_len())));
    parts.push(format!(
        "modlog_ch/entries={}",
        pair(crate::services::mod_log_storage_service::cache_counts())
    ));
    parts.push(format!("7tv_subs={}", n(crate::services::seventv_eventapi::sub_count())));
    format!("caches {}", parts.join(" "))
}

/// Walk the snapshot from `self_pid` down and classify every descendant.
/// Pure, so it is unit-tested without Win32.
pub fn build_tree(self_pid: u32, entries: &[Entry], kinds: &HashMap<u32, Kind>) -> Vec<(Entry, Kind)> {
    let mut members: HashMap<u32, Kind> = HashMap::new();
    let mut ordered: Vec<(Entry, Kind)> = Vec::new();
    if let Some(me) = entries.iter().find(|e| e.pid == self_pid) {
        members.insert(self_pid, Kind::Rust);
        ordered.push((me.clone(), Kind::Rust));
    } else {
        return ordered;
    }
    // Children can appear before their parents in a snapshot, so iterate until
    // a pass adds nothing. The tree is a dozen processes; this is cheap.
    loop {
        let mut added = false;
        for e in entries {
            if e.pid == self_pid || members.contains_key(&e.pid) || !members.contains_key(&e.parent) {
                continue;
            }
            let kind = classify(e, self_pid, kinds);
            members.insert(e.pid, kind);
            ordered.push((e.clone(), kind));
            added = true;
        }
        if !added {
            break;
        }
    }
    ordered
}

fn classify(e: &Entry, self_pid: u32, kinds: &HashMap<u32, Kind>) -> Kind {
    if e.name.eq_ignore_ascii_case("msedgewebview2") {
        return kinds.get(&e.pid).copied().unwrap_or(Kind::WebViewOther);
    }
    if e.parent == self_pid {
        Kind::Plugin
    } else {
        Kind::Other
    }
}

/// Render the rows as the one-line summary. Order is fixed so lines diff
/// cleanly across a session: rust, browser, gpu, renderers, utilities, other
/// WebView2 processes, plugins, everything else.
pub fn format_line(rows: &[ProcRow]) -> String {
    let total_ws: u64 = rows.iter().map(|r| r.ws_mb).sum();
    let total_private: u64 = rows.iter().map(|r| r.private_mb).sum();
    let mut parts: Vec<String> = vec![format!("tree ws={total_ws} priv={total_private}")];

    let singles = [(Kind::Rust, "rust"), (Kind::Browser, "browser"), (Kind::Gpu, "gpu")];
    for (kind, label) in singles {
        let matching: Vec<&ProcRow> = rows.iter().filter(|r| r.kind == kind).collect();
        let tag_pid = matching.len() > 1;
        for r in matching {
            let tag = if tag_pid { format!("{label}#{}", r.pid) } else { label.to_string() };
            parts.push(format!("{tag} {}/{} h{} t{}", r.ws_mb, r.private_mb, r.handles, r.threads));
        }
    }
    let mut renderers: Vec<&ProcRow> = rows.iter().filter(|r| r.kind == Kind::Renderer).collect();
    renderers.sort_by_key(|r| r.pid);
    for r in renderers {
        parts.push(format!("renderer#{} {}/{} h{} t{}", r.pid, r.ws_mb, r.private_mb, r.handles, r.threads));
    }
    for (kind, label) in [(Kind::Utility, "utility"), (Kind::WebViewOther, "wv2-other")] {
        let matching: Vec<&ProcRow> = rows.iter().filter(|r| r.kind == kind).collect();
        if matching.is_empty() {
            continue;
        }
        let ws: u64 = matching.iter().map(|r| r.ws_mb).sum();
        let private: u64 = matching.iter().map(|r| r.private_mb).sum();
        parts.push(format!("{label}[{}] {ws}/{private}", matching.len()));
    }
    let mut plugins: Vec<&ProcRow> = rows.iter().filter(|r| r.kind == Kind::Plugin).collect();
    plugins.sort_by(|a, b| a.name.cmp(&b.name).then(a.pid.cmp(&b.pid)));
    for r in plugins {
        parts.push(format!("plugin {} {}/{}", r.name, r.ws_mb, r.private_mb));
    }
    let others: Vec<&ProcRow> = rows.iter().filter(|r| r.kind == Kind::Other).collect();
    if !others.is_empty() {
        let ws: u64 = others.iter().map(|r| r.ws_mb).sum();
        let private: u64 = others.iter().map(|r| r.private_mb).sum();
        parts.push(format!("other[{}] {ws}/{private}", others.len()));
    }
    parts.join(" | ")
}

fn strip_exe(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    match lower.strip_suffix(".exe") {
        Some(base) => base.to_string(),
        None => lower,
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::mpsc;

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Environment, ICoreWebView2Environment8, COREWEBVIEW2_PROCESS_KIND_BROWSER,
        COREWEBVIEW2_PROCESS_KIND_GPU, COREWEBVIEW2_PROCESS_KIND_RENDERER,
        COREWEBVIEW2_PROCESS_KIND_UTILITY,
    };
    use windows::core::Interface;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetProcessHandleCount, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// Ask WebView2 which pid is which kind. Runs on the UI thread through
    /// `with_webview` (any window will do: they share one environment), with
    /// a bounded wait. Empty when no window exists or the UI thread did not
    /// answer in time; the caller then logs the tree unclassified.
    pub fn webview_kinds(app: &AppHandle) -> HashMap<u32, Kind> {
        let Some(window) = app.webview_windows().into_values().next() else {
            return HashMap::new();
        };
        let (tx, rx) = mpsc::channel::<HashMap<u32, Kind>>();
        let dispatched = window.with_webview(move |platform_webview| {
            let kinds = unsafe { query_kinds(platform_webview.environment()) }.unwrap_or_default();
            let _ = tx.send(kinds);
        });
        if dispatched.is_err() {
            return HashMap::new();
        }
        rx.recv_timeout(KIND_QUERY_TIMEOUT).unwrap_or_default()
    }

    unsafe fn query_kinds(env: ICoreWebView2Environment) -> windows::core::Result<HashMap<u32, Kind>> {
        let env8: ICoreWebView2Environment8 = env.cast()?;
        let infos = env8.GetProcessInfos()?;
        let mut count = 0u32;
        infos.Count(&mut count)?;
        let mut out = HashMap::with_capacity(count as usize);
        for i in 0..count {
            let info = infos.GetValueAtIndex(i)?;
            let mut pid = 0i32;
            info.ProcessId(&mut pid)?;
            let mut kind = Default::default();
            info.Kind(&mut kind)?;
            let mapped = match kind {
                k if k == COREWEBVIEW2_PROCESS_KIND_BROWSER => Kind::Browser,
                k if k == COREWEBVIEW2_PROCESS_KIND_GPU => Kind::Gpu,
                k if k == COREWEBVIEW2_PROCESS_KIND_RENDERER => Kind::Renderer,
                k if k == COREWEBVIEW2_PROCESS_KIND_UTILITY => Kind::Utility,
                _ => Kind::WebViewOther,
            };
            if pid > 0 {
                out.insert(pid as u32, mapped);
            }
        }
        Ok(out)
    }

    /// Snapshot every process, keep our tree, read memory for each member.
    pub fn sample_tree(kinds: &HashMap<u32, Kind>) -> Option<Vec<ProcRow>> {
        let self_pid = unsafe { GetCurrentProcessId() };
        let entries = snapshot().ok()?;
        let tree = build_tree(self_pid, &entries, kinds);
        let rows = tree
            .into_iter()
            .map(|(e, kind)| {
                let (ws, private, handles) = memory_of(e.pid).unwrap_or((0, 0, 0));
                ProcRow {
                    pid: e.pid,
                    name: e.name,
                    kind,
                    ws_mb: ws / (1024 * 1024),
                    private_mb: private / (1024 * 1024),
                    handles,
                    threads: e.threads,
                }
            })
            .collect();
        Some(rows)
    }

    fn snapshot() -> windows::core::Result<Vec<Entry>> {
        let mut out = Vec::with_capacity(256);
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            let mut pe = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut ok = Process32FirstW(snap, &mut pe).is_ok();
            while ok {
                let len = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
                let name = String::from_utf16_lossy(&pe.szExeFile[..len]);
                out.push(Entry {
                    pid: pe.th32ProcessID,
                    parent: pe.th32ParentProcessID,
                    threads: pe.cntThreads,
                    name: strip_exe(&name),
                });
                ok = Process32NextW(snap, &mut pe).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        Ok(out)
    }

    /// (working set bytes, private bytes, handle count).
    fn memory_of(pid: u32) -> Option<(u64, u64, u32)> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut counters = PROCESS_MEMORY_COUNTERS_EX {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
                ..Default::default()
            };
            let ok = K32GetProcessMemoryInfo(
                handle,
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                counters.cb,
            )
            .as_bool();
            let mut handles = 0u32;
            let _ = GetProcessHandleCount(handle, &mut handles);
            let _ = CloseHandle(handle);
            if !ok {
                return None;
            }
            Some((counters.WorkingSetSize as u64, counters.PrivateUsage as u64, handles))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn webview_kinds(_app: &AppHandle) -> HashMap<u32, Kind> {
        HashMap::new()
    }

    pub fn sample_tree(_kinds: &HashMap<u32, Kind>) -> Option<Vec<ProcRow>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(pid: u32, parent: u32, name: &str) -> Entry {
        Entry { pid, parent, threads: 1, name: name.to_string() }
    }

    fn row(pid: u32, name: &str, kind: Kind, ws: u64, private: u64) -> ProcRow {
        ProcRow { pid, name: name.into(), kind, ws_mb: ws, private_mb: private, handles: 10, threads: 2 }
    }

    #[test]
    fn tree_walks_descendants_and_classifies_them() {
        // Children listed before parents, an unrelated process, a plugin with
        // a conhost child, and WebView2 processes with and without a kind.
        let entries = vec![
            e(300, 200, "msedgewebview2"),
            e(999, 1, "explorer"),
            e(200, 100, "msedgewebview2"),
            e(100, 1, "streamnook"),
            e(400, 100, "drops-farmer"),
            e(401, 400, "conhost"),
            e(301, 200, "msedgewebview2"),
        ];
        let kinds: HashMap<u32, Kind> =
            [(200, Kind::Browser), (300, Kind::Renderer)].into_iter().collect();
        let tree = build_tree(100, &entries, &kinds);
        let by_pid: HashMap<u32, Kind> = tree.iter().map(|(e, k)| (e.pid, *k)).collect();
        assert_eq!(by_pid.len(), 6);
        assert_eq!(by_pid[&100], Kind::Rust);
        assert_eq!(by_pid[&200], Kind::Browser);
        assert_eq!(by_pid[&300], Kind::Renderer);
        assert_eq!(by_pid[&301], Kind::WebViewOther);
        assert_eq!(by_pid[&400], Kind::Plugin);
        assert_eq!(by_pid[&401], Kind::Other);
        assert!(!by_pid.contains_key(&999));
    }

    #[test]
    fn tree_is_empty_when_self_is_missing() {
        assert!(build_tree(7, &[e(1, 0, "x")], &HashMap::new()).is_empty());
    }

    #[test]
    fn line_has_fixed_order_and_sums() {
        let rows = vec![
            row(400, "drops-farmer", Kind::Plugin, 16, 6),
            row(302, "msedgewebview2", Kind::Utility, 48, 15),
            row(300, "msedgewebview2", Kind::Renderer, 387, 299),
            row(100, "streamnook", Kind::Rust, 203, 163),
            row(303, "msedgewebview2", Kind::Utility, 26, 14),
            row(201, "msedgewebview2", Kind::Gpu, 218, 269),
            row(200, "msedgewebview2", Kind::Browser, 154, 39),
            row(399, "ad-bypass", Kind::Plugin, 16, 6),
            row(401, "conhost", Kind::Other, 7, 1),
            row(304, "msedgewebview2", Kind::WebViewOther, 14, 3),
        ];
        let line = format_line(&rows);
        assert_eq!(
            line,
            "tree ws=1089 priv=815 | rust 203/163 h10 t2 | browser 154/39 h10 t2 | gpu 218/269 h10 t2 \
             | renderer#300 387/299 h10 t2 | utility[2] 74/29 | wv2-other[1] 14/3 \
             | plugin ad-bypass 16/6 | plugin drops-farmer 16/6 | other[1] 7/1"
        );
    }

    #[test]
    fn two_renderers_are_listed_separately_by_pid() {
        let rows = vec![
            row(1, "streamnook", Kind::Rust, 1, 1),
            row(30, "msedgewebview2", Kind::Renderer, 5, 5),
            row(20, "msedgewebview2", Kind::Renderer, 4, 4),
        ];
        let line = format_line(&rows);
        assert!(line.contains("renderer#20 4/4 h10 t2 | renderer#30 5/5 h10 t2"), "{line}");
    }

    /// The real Win32 path on the test process: the snapshot must find us,
    /// classify us as Rust, and read non-zero counters. No WebView2 here, so
    /// the kind map is empty and no child is expected.
    #[cfg(windows)]
    #[test]
    fn live_sample_reads_own_process() {
        let rows = imp::sample_tree(&HashMap::new()).expect("snapshot");
        let me = rows.iter().find(|r| r.kind == Kind::Rust).expect("own process in tree");
        assert!(me.ws_mb > 0, "working set should be non-zero: {me:?}");
        assert!(me.private_mb > 0, "private bytes should be non-zero: {me:?}");
        assert!(me.handles > 0 && me.threads > 0, "{me:?}");
        assert_eq!(me.pid, std::process::id());
        let line = format_line(&rows);
        assert!(line.starts_with("tree ws="), "{line}");
    }

    #[test]
    fn exe_suffix_is_stripped_case_insensitively() {
        assert_eq!(strip_exe("StreamNook.EXE"), "streamnook");
        assert_eq!(strip_exe("conhost"), "conhost");
    }
}
