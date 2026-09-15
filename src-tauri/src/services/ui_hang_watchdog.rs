//! UI-thread "Not Responding" detector.
//!
//! "Not Responding" is the Win32 UI thread (the window's message pump) failing to
//! pump messages, NOT a CPU/tokio stall. `runtime_watchdog` measures tokio and
//! whole-process scheduling lag; it cannot see a pump that is wedged while every
//! other thread keeps running, which is the classic shape of a blocking WebView2 /
//! COM call (e.g. native window-occlusion handling when another window covers ours
//! during the Twitch login overlay).
//!
//! This watchdog probes the pump directly, the same way the OS decides whether to
//! paint "(Not Responding)", and records onset/recovery with enough context to name
//! the real cause: what window took the foreground, whether ours was minimized, which
//! Twitch overlay was mounted, and whether process scheduling was still healthy at
//! the time (small self-lag + hung pump == pump wedge, not CPU).
//!
//! The capture path is allocation-free on purpose. A wedged UI thread can be stuck
//! inside an allocation or a COM call that holds the process heap lock; anything on
//! this thread that allocates (format!, String, the log crate, std::fs path joins)
//! would then block on the same lock and the hang would go unrecorded. So a hang is
//! formatted into a fixed stack buffer and written with raw CreateFileW/WriteFile
//! (kernel calls that use kernel pools, not the process heap). The log path is
//! pre-encoded to UTF-16 once at startup, the only heap work this module does.
//!
//! Reports land in the same `logs/errors.log` the crash logger uses, so its presence
//! after a freeze (or its absence) is a signal in itself. A separate `logs/
//! ui_hang_watchdog.armed` marker is written once at startup so an absent errors.log
//! can be read as "started, never hung" rather than "never started / old build".

/// Master switch. On by default: the probe is one message per second and costs
/// nothing until something hangs. Flip to `false` + rebuild to disable.
const ENABLED: bool = true;

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use windows::core::{BOOL, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, WriteFile, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_ALWAYS,
    };
    use windows::Win32::System::SystemInformation::GetSystemTime;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumThreadWindows, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
        GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        SendMessageTimeoutW, GUITHREADINFO, GUI_INMENUMODE, GUI_INMOVESIZE, GUI_POPUPMENUMODE,
        GUI_SYSTEMMENUMODE, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_NULL,
    };

    // Probe cadence when healthy, and the blocking timeout that defines "the pump
    // didn't answer". A genuine "Not Responding" is multi-second; ordinary work
    // never blocks WM_NULL for two seconds.
    const HEALTHY_INTERVAL: Duration = Duration::from_millis(1000);
    const PROBE_TIMEOUT_MS: u32 = 2000;
    // Don't log brief 2-4s blips; only sustained hangs the user actually sees as
    // "(Not Responding)" (Windows ghosts a window at ~5s).
    const REPORT_AFTER: Duration = Duration::from_millis(4000);
    // Below this, the watchdog thread itself was scheduled on time, so a hung pump
    // means the UI thread is wedged rather than the whole process starving.
    const SELF_LAG_OK_MS: u128 = 500;

    static ACTIVE_OVERLAY: Mutex<Option<String>> = Mutex::new(None);
    /// The overlay that was up last time `ACTIVE_OVERLAY` went to `None`, with when it
    /// was cleared. Lets a hang that fires right after an overlay tears down still name
    /// it ("none (last: twitch-login@..., 800ms ago)"), which the first captures could
    /// not do — they only showed `none`.
    static LAST_OVERLAY: Mutex<Option<(String, Instant)>> = Mutex::new(None);
    /// `logs/errors.log` pre-encoded as a NUL-terminated UTF-16 string. Resolved once
    /// at startup so the hang path never allocates or joins paths.
    static LOG_PATH_W: OnceLock<Vec<u16>> = OnceLock::new();

    pub fn set_active_overlay(ctx: Option<String>) {
        if let Ok(mut g) = ACTIVE_OVERLAY.lock() {
            // On clear, remember what was showing and when, so a just-after-dismiss hang
            // still has the context (the cloning + Instant are off the hang path).
            if ctx.is_none() {
                if let Some(prev) = g.as_ref() {
                    if let Ok(mut last) = LAST_OVERLAY.lock() {
                        *last = Some((prev.clone(), Instant::now()));
                    }
                }
            }
            *g = ctx;
        }
    }

    /// A small fixed-size buffer formatted into on the stack. Writes past capacity are
    /// dropped (truncated), never reallocated, so a report can never allocate.
    struct StackBuf {
        buf: [u8; 4096],
        len: usize,
    }

    impl StackBuf {
        const fn new() -> Self {
            Self {
                buf: [0u8; 4096],
                len: 0,
            }
        }

        fn bytes(&self) -> &[u8] {
            &self.buf[..self.len]
        }

        fn push(&mut self, b: &[u8]) {
            let avail = self.buf.len() - self.len;
            let n = if b.len() < avail { b.len() } else { avail };
            self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
            self.len += n;
        }

        /// Append a UTF-16 slice as UTF-8 without allocating (one char at a time into
        /// a 4-byte stack scratch).
        fn push_u16_lossy(&mut self, w: &[u16]) {
            let mut tmp = [0u8; 4];
            for ch in core::char::decode_utf16(w.iter().copied()) {
                let c = ch.unwrap_or('\u{FFFD}');
                self.push(c.encode_utf8(&mut tmp).as_bytes());
            }
        }
    }

    // write!(...) into the buffer formats integers/strings directly into write_str,
    // which never allocates.
    impl core::fmt::Write for StackBuf {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.push(s.as_bytes());
            Ok(())
        }
    }

    /// UTC `YYYY-MM-DD HH:MM:SSZ` from GetSystemTime (no chrono, no allocation).
    fn push_timestamp(b: &mut StackBuf) {
        use core::fmt::Write as _;
        let t = unsafe { GetSystemTime() };
        let _ = write!(
            b,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        );
    }

    /// Title + executable of whatever window currently holds the foreground, appended
    /// in place. During a reported hang this is almost always what the user alt-tabbed
    /// to. Safe to call: the foreground window is by definition the responsive one, so
    /// its pump answers immediately (we never query our own wedged window here).
    fn push_foreground(b: &mut StackBuf) {
        let before = b.len;
        unsafe {
            let fg = GetForegroundWindow();
            if fg.0.is_null() {
                b.push(b"unknown");
                return;
            }
            let mut title = [0u16; 256];
            let n = GetWindowTextW(fg, &mut title);
            if n > 0 {
                b.push(b"\"");
                b.push_u16_lossy(&title[..n as usize]);
                b.push(b"\" ");
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(fg, Some(&mut pid));
            push_process_exe(b, pid);
        }
        if b.len == before {
            b.push(b"unknown");
        }
    }

    fn push_process_exe(b: &mut StackBuf, pid: u32) {
        if pid == 0 {
            return;
        }
        unsafe {
            let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return;
            };
            let mut buf = [0u16; 260];
            let mut len = buf.len() as u32;
            let res =
                QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
            let _ = CloseHandle(handle);
            if res.is_err() {
                return;
            }
            let full = &buf[..len as usize];
            // Last path component (the bare exe name).
            let start = full
                .iter()
                .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
                .map(|i| i + 1)
                .unwrap_or(0);
            b.push(b"(");
            b.push_u16_lossy(&full[start..]);
            b.push(b")");
        }
    }

    /// Read the mounted-overlay context with try_lock so a (theoretically) stuck lock
    /// can never wedge the watchdog, and without cloning the String.
    fn push_overlay_ctx(b: &mut StackBuf) {
        use core::fmt::Write as _;
        match ACTIVE_OVERLAY.try_lock() {
            Ok(g) => match g.as_deref() {
                Some(s) => b.push(s.as_bytes()),
                None => {
                    b.push(b"none");
                    // Name the overlay that most recently closed, if any, so a hang right
                    // after teardown is not mistaken for "no overlay involved".
                    if let Ok(last) = LAST_OVERLAY.try_lock() {
                        if let Some((label, when)) = last.as_ref() {
                            b.push(b" (last: ");
                            b.push(label.as_bytes());
                            let _ = write!(b, ", {}ms ago)", when.elapsed().as_millis());
                        }
                    }
                }
            },
            Err(_) => b.push(b"locked"),
        }
    }

    /// One window per line, for every top-level window the UI thread owns at hang time:
    /// class name, minimized/visible, count of WebView2 child surfaces (`Chrome_*`), and
    /// screen rect. This is the inventory the first captures lacked: it shows which
    /// window (main vs an overlay vs a hidden scraper) and which WebView2 was actually
    /// live when the pump wedged, independent of the `ACTIVE_OVERLAY` tracker. Every call
    /// here reads cached window-manager state (no `WM_GETTEXT`/`SendMessage`), so it can
    /// never block on the wedged thread.
    const MAX_INV_WINDOWS: u32 = 16;

    struct InvCtx {
        buf: *mut StackBuf,
        count: u32,
    }

    unsafe extern "system" fn inventory_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        use core::fmt::Write as _;
        let ctx = &mut *(lparam.0 as *mut InvCtx);
        if ctx.count >= MAX_INV_WINDOWS {
            return BOOL(0); // stop enumerating
        }
        ctx.count += 1;
        let b = &mut *ctx.buf;

        let mut cls = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut cls);
        let iconic = IsIconic(hwnd).as_bool();
        let visible = IsWindowVisible(hwnd).as_bool();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = GetWindowRect(hwnd, &mut rect);

        let mut wv: u32 = 0;
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(webview_child_cb),
            LPARAM(&mut wv as *mut u32 as isize),
        );

        b.push(b"\n  - ");
        if n > 0 {
            b.push_u16_lossy(&cls[..n as usize]);
        } else {
            b.push(b"?");
        }
        let _ = write!(
            b,
            " iconic={iconic} vis={visible} wv={wv} rect=({},{} {}x{})",
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top
        );
        BOOL(1)
    }

    /// Counts `Chrome_*` descendants (the WebView2 host/render surfaces) under a window.
    unsafe extern "system" fn webview_child_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let counter = &mut *(lparam.0 as *mut u32);
        let mut cls = [0u16; 32];
        let n = GetClassNameW(hwnd, &mut cls);
        if n > 0 {
            const PFX: [u16; 7] = [
                b'C' as u16,
                b'h' as u16,
                b'r' as u16,
                b'o' as u16,
                b'm' as u16,
                b'e' as u16,
                b'_' as u16,
            ];
            if cls[..n as usize].starts_with(&PFX) {
                *counter += 1;
            }
        }
        BOOL(1)
    }

    fn push_window_inventory(b: &mut StackBuf, ui_tid: u32) {
        b.push(b"\nwindows (UI-thread top-level):");
        // Reborrow to a raw pointer so `b` is free again after the (synchronous) enum.
        let buf_ptr: *mut StackBuf = &mut *b;
        let mut ctx = InvCtx {
            buf: buf_ptr,
            count: 0,
        };
        unsafe {
            let _ = EnumThreadWindows(
                ui_tid,
                Some(inventory_cb),
                LPARAM(&mut ctx as *mut InvCtx as isize),
            );
        }
        if ctx.count >= MAX_INV_WINDOWS {
            b.push(b"\n  - (more, truncated)");
        } else if ctx.count == 0 {
            b.push(b" none");
        }
    }

    /// Win32 input state of the wedged UI thread: whether it is sitting in a modal
    /// move/size or menu message loop (a classic "blocking native call" shape), plus the
    /// move/size and menu-owner windows if so. Reads kernel-side thread state, no message
    /// sent, so it is safe against the hang.
    fn push_gui_thread_info(b: &mut StackBuf, ui_tid: u32) {
        use core::fmt::Write as _;
        let mut gti: GUITHREADINFO = unsafe { core::mem::zeroed() };
        gti.cbSize = core::mem::size_of::<GUITHREADINFO>() as u32;
        b.push(b"\ngui-thread: ");
        if unsafe { GetGUIThreadInfo(ui_tid, &mut gti) }.is_err() {
            b.push(b"(unavailable)");
            return;
        }
        let f = gti.flags;
        let mut any = false;
        if f.contains(GUI_INMOVESIZE) {
            b.push(b"move/size-loop ");
            any = true;
        }
        if f.contains(GUI_INMENUMODE) {
            b.push(b"menu-mode ");
            any = true;
        }
        if f.contains(GUI_POPUPMENUMODE) {
            b.push(b"popup-menu ");
            any = true;
        }
        if f.contains(GUI_SYSTEMMENUMODE) {
            b.push(b"system-menu ");
            any = true;
        }
        if !any {
            b.push(b"no-modal-loop ");
        }
        let _ = write!(
            b,
            "moveSize_hwnd={:#x} menuOwner_hwnd={:#x}",
            gti.hwndMoveSize.0 as usize, gti.hwndMenuOwner.0 as usize
        );
    }

    fn pump_responsive(hwnd: HWND) -> bool {
        unsafe {
            // WM_NULL does nothing but must be dispatched by the pump, so a reply
            // means the UI thread is pumping. SMTO_ABORTIFHUNG returns at once if the
            // window is already ghosted; SMTO_BLOCK keeps the probe self-contained.
            let r = SendMessageTimeoutW(
                hwnd,
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                PROBE_TIMEOUT_MS,
                None,
            );
            r.0 != 0
        }
    }

    fn window_state(hwnd: HWND) -> (bool, bool) {
        unsafe {
            let minimized = IsIconic(hwnd).as_bool();
            let focused = GetForegroundWindow() == hwnd;
            (minimized, focused)
        }
    }

    /// Resolve and cache the errors.log path (UTF-16), create the logs dir, and drop a
    /// one-shot "armed" marker. The only heap work in this module, done at startup so
    /// the hang path stays allocation-free. Idempotent.
    fn init_paths() {
        if LOG_PATH_W.get().is_some() {
            return;
        }
        let Ok(dir) = crate::services::cache_service::get_app_data_dir() else {
            return;
        };
        let logs = dir.join("logs");
        let _ = std::fs::create_dir_all(&logs);
        write_armed_marker(&logs);

        use std::os::windows::ffi::OsStrExt;
        let path = logs.join("errors.log");
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let _ = LOG_PATH_W.set(wide);
    }

    /// Overwritten each launch in its own file (not errors.log, which stays a pure
    /// hang signal). Its presence proves the watchdog reached startup with a valid
    /// HWND; an absent errors.log alongside a present marker means "ran, never hung".
    fn write_armed_marker(logs: &std::path::Path) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::File::create(logs.join("ui_hang_watchdog.armed")) {
            let _ = writeln!(
                f,
                "ui-hang-watchdog armed: app {} pid {}",
                env!("CARGO_PKG_VERSION"),
                std::process::id()
            );
        }
    }

    /// Append `bytes` to errors.log with raw Win32 calls only (no Rust-heap
    /// allocation), so a report survives even a process-heap-locked freeze. No-op if
    /// the path wasn't resolved at startup.
    fn write_block(bytes: &[u8]) {
        let Some(path) = LOG_PATH_W.get() else {
            return;
        };
        unsafe {
            // FILE_APPEND_DATA (without FILE_WRITE_DATA) makes every write land at EOF,
            // so no seek is needed and concurrent writers (the crash logger) interleave
            // cleanly. OPEN_ALWAYS creates the file on first hang.
            let handle = match CreateFileW(
                PCWSTR(path.as_ptr()),
                FILE_APPEND_DATA.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                None,
            ) {
                Ok(h) => h,
                Err(_) => return,
            };
            let mut written = 0u32;
            let _ = WriteFile(handle, Some(bytes), Some(&mut written), None);
            let _ = CloseHandle(handle);
        }
    }

    pub fn start(hwnd_raw: isize) {
        init_paths();
        std::thread::Builder::new()
            .name("ui-hang-watchdog".into())
            .spawn(move || {
                let hwnd = HWND(hwnd_raw as *mut c_void);
                let mut hang_start: Option<Instant> = None;
                let mut reported = false;
                let mut self_lag_ms: u128 = 0;
                loop {
                    // The main window can be destroyed at runtime (going live closes
                    // it to free its memory). A probe against a dead HWND returns
                    // instantly with "no answer", which would spin this loop hot and
                    // false-report a hang. Detect the gone window and end the thread;
                    // a fresh watchdog is started when the main window is recreated.
                    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                        return;
                    }
                    if pump_responsive(hwnd) {
                        if let Some(start) = hang_start.take() {
                            if reported {
                                report_recovery(start.elapsed().as_millis());
                            }
                            reported = false;
                        }
                        // Healthy: sleep, and measure our own scheduling lag so a
                        // hang report can say whether the process was also starving.
                        let t0 = Instant::now();
                        std::thread::sleep(HEALTHY_INTERVAL);
                        self_lag_ms = t0.elapsed().saturating_sub(HEALTHY_INTERVAL).as_millis();
                        continue;
                    }
                    // Pump did not answer within PROBE_TIMEOUT_MS.
                    let start = *hang_start.get_or_insert_with(Instant::now);
                    if !reported && start.elapsed() >= REPORT_AFTER {
                        report_onset(hwnd, start.elapsed().as_millis(), self_lag_ms);
                        reported = true;
                    }
                    // While hung the probe itself blocks ~PROBE_TIMEOUT_MS, so don't
                    // add a sleep; just keep re-probing to catch the recovery edge.
                }
            })
            .expect("spawn ui-hang-watchdog thread");
    }

    fn report_onset(hwnd: HWND, elapsed_ms: u128, self_lag_ms: u128) {
        use core::fmt::Write as _;
        let (minimized, focused) = window_state(hwnd);
        let verdict = if self_lag_ms < SELF_LAG_OK_MS {
            "process scheduling normal -> UI-thread / message-pump wedge (blocking native or COM call), not CPU saturation"
        } else {
            "process scheduling also lagging -> whole-process stall (CPU / swap); see runtime_watchdog"
        };
        let mut b = StackBuf::new();
        b.push(b"\n========== ");
        push_timestamp(&mut b);
        let _ = writeln!(
            b,
            " | app {} {} ==========",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        );
        let _ = writeln!(
            b,
            "ERROR [UIHang] UI thread unresponsive (Not Responding), ongoing ~{elapsed_ms}ms"
        );
        b.push(b"foreground: ");
        push_foreground(&mut b);
        let _ = write!(
            b,
            "\nour window: minimized={minimized} focused={focused}\noverlay mounted: "
        );
        push_overlay_ctx(&mut b);
        let _ = writeln!(
            b,
            "\nprobe: SendMessageTimeout(WM_NULL) exceeded {PROBE_TIMEOUT_MS}ms\n\
             process scheduling lag (watchdog self): {self_lag_ms}ms\n\
             verdict: {verdict}"
        );
        // Detail section appended AFTER the verdict, so if the inventory ever fills the
        // buffer the truncation eats the detail, never the summary or verdict above.
        let ui_tid = unsafe { GetWindowThreadProcessId(hwnd, None) };
        push_gui_thread_info(&mut b, ui_tid);
        push_window_inventory(&mut b, ui_tid);
        b.push(b"\n");
        write_block(b.bytes());
    }

    fn report_recovery(total_ms: u128) {
        use core::fmt::Write as _;
        let mut b = StackBuf::new();
        b.push(b"\n========== ");
        push_timestamp(&mut b);
        let _ = writeln!(
            b,
            " | app {} {} ==========",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        );
        let _ = writeln!(b, "ERROR [UIHang] UI thread recovered after ~{total_ms}ms");
        write_block(b.bytes());
    }
}

/// Record which Twitch overlay webview is mounted, so a hang report names what was
/// on screen. Set when the overlay's native child webview mounts, cleared on dismiss.
#[cfg(windows)]
pub use imp::set_active_overlay;

/// Spawn the watchdog for the main window's HWND (passed as a raw value so nothing
/// `!Send` crosses the thread boundary). Call once from Tauri `setup`.
#[cfg(windows)]
pub fn start_for_hwnd(hwnd_raw: isize) {
    if ENABLED {
        imp::start(hwnd_raw);
    }
}

// ---------------------------------------------------------------------------
// Non-Windows: same PURPOSE, different mechanism.
//
// Everything above probes the Win32 message pump, which is how Windows itself
// decides whether to paint "(Not Responding)". There is no such thing to probe
// on macOS or Linux, so this is not a port — it is a second implementation of
// the same intent, built on a main-thread round-trip. See
// `platform::responsiveness` for why that catches the same class of fault.
//
// These were previously empty stubs, and both `start_for_hwnd` call sites in
// main.rs are `#[cfg(windows)]`, so off Windows the app simply had NO hang
// detection at all — silently.
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
mod portable {
    use crate::platform::responsiveness::{
        probe_main_thread, HangState, Probe, Transition,
    };
    use std::sync::Mutex;
    use std::sync::OnceLock;
    use std::time::Duration;

    /// How long the main thread gets to answer before a probe counts as missed.
    /// Generous on purpose: a legitimate synchronous layout or a native file
    /// dialog can hold the thread for a while without being wedged, and
    /// `HangState` needs several consecutive misses anyway.
    const PROBE_DEADLINE: Duration = Duration::from_millis(2500);
    const PROBE_INTERVAL: Duration = Duration::from_secs(1);

    fn overlay_ctx() -> &'static Mutex<Option<String>> {
        static CTX: OnceLock<Mutex<Option<String>>> = OnceLock::new();
        CTX.get_or_init(|| Mutex::new(None))
    }

    /// `<app_data>/logs`, where streamnook.log, errors.log and the samples
    /// live. `None` only if the data dir cannot be resolved at all.
    fn logs_dir() -> Option<std::path::PathBuf> {
        crate::services::file_log::log_file_path()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// How many `sample` captures one session may write. A wedge that never
    /// recovers produces exactly one; the cap only stops a flapping UI thread
    /// from filling the logs folder.
    #[cfg(target_os = "macos")]
    const MAX_SAMPLES_PER_SESSION: u32 = 3;

    /// Capture the wedged process's call stacks with macOS's own `sample`
    /// tool, into the logs folder next to streamnook.log.
    ///
    /// The Windows watchdog can name the window and the foreground app at
    /// hang time. On macOS the only thing that answers "what is the main
    /// thread stuck in" is a stack sample, and the first macOS freeze report
    /// (8.6.2) came with no way to take one after the fact. So the app
    /// samples itself: `/usr/bin/sample` ships with the OS, works on any
    /// process the user owns (this bundle is not hardened), and the release
    /// binary keeps its symbol table (`strip = "debuginfo"` in Cargo.toml) so
    /// the frames read as Rust function names rather than addresses.
    ///
    /// Runs on its own thread so probing continues and recovery is still
    /// timed. Never touches the wedged thread. `-mayDie` keeps a Force Quit
    /// mid-sample from turning into an error. `sample` ends on its own after
    /// the requested seconds, so the wait is bounded.
    #[cfg(target_os = "macos")]
    fn sample_main_thread(missed: u32) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CAPTURED: AtomicU32 = AtomicU32::new(0);
        let n = CAPTURED.fetch_add(1, Ordering::Relaxed) + 1;
        if n > MAX_SAMPLES_PER_SESSION {
            return;
        }
        let Some(dir) = logs_dir() else {
            log::warn!("[UiHang] no logs dir; cannot write a main-thread sample");
            return;
        };
        let out = dir.join(format!(
            "ui-hang-{}.txt",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        ));
        let _ = std::thread::Builder::new()
            .name("ui-hang-sampler".into())
            .spawn(move || {
                let pid = std::process::id().to_string();
                let result = std::process::Command::new("/usr/bin/sample")
                    .args([pid.as_str(), "3", "-mayDie", "-file"])
                    .arg(&out)
                    .stdin(std::process::Stdio::null())
                    .output();
                match result {
                    Ok(o) if o.status.success() && out.exists() => log::error!(
                        "[UiHang] main-thread sample #{n} written to {} after {missed} \
                         missed probes; attach it to the bug report",
                        out.display()
                    ),
                    Ok(o) => log::warn!(
                        "[UiHang] sample exited with {}: {}",
                        o.status,
                        String::from_utf8_lossy(&o.stderr).trim()
                    ),
                    Err(e) => log::warn!("[UiHang] could not run /usr/bin/sample: {e}"),
                }
            });
    }

    /// Record which overlay is mounted, so a hang report can name it. This is
    /// the same context the Windows reporter captures; without it a freeze
    /// report cannot distinguish "wedged during the Twitch login overlay" from
    /// "wedged at idle".
    pub fn set_active_overlay(ctx: Option<String>) {
        if let Ok(mut slot) = overlay_ctx().lock() {
            *slot = ctx;
        }
    }

    pub fn start(app: tauri::AppHandle) {
        // Same marker the Windows watchdog writes: an absent hang report can
        // then be read as "armed, never hung" rather than "never started".
        if let Some(dir) = logs_dir() {
            let _ = std::fs::write(dir.join("ui_hang_watchdog.armed"), b"");
        }
        std::thread::Builder::new()
            .name("ui-hang-watchdog".into())
            .spawn(move || {
                let mut state = HangState::new();
                loop {
                    std::thread::sleep(PROBE_INTERVAL);
                    let probe = probe_main_thread(&app, PROBE_DEADLINE);
                    match state.observe(probe) {
                        Transition::Steady => {}
                        Transition::HangStarted { missed } => {
                            crate::platform::responsiveness::record_hang();
                            let ctx = overlay_ctx()
                                .lock()
                                .ok()
                                .and_then(|c| c.clone())
                                .unwrap_or_else(|| "none".into());
                            log::error!(
                                "[UiHang] UI thread unresponsive: {missed} consecutive \
                                 missed probes ({}ms deadline each), overlay = {ctx}",
                                PROBE_DEADLINE.as_millis()
                            );
                            #[cfg(target_os = "macos")]
                            sample_main_thread(missed);
                        }
                        Transition::Recovered { missed } => {
                            log::warn!(
                                "[UiHang] UI thread recovered after {missed} missed \
                                 probes (~{}s)",
                                missed as u64 * PROBE_INTERVAL.as_secs()
                            );
                        }
                    }
                    let _ = probe;
                    if matches!(probe, Probe::Responsive) && state.is_hung() {
                        // Defensive: observe() should already have cleared this.
                        debug_assert!(false, "state left hung after a responsive probe");
                    }
                }
            })
            .ok();
    }
}

#[cfg(not(windows))]
pub fn set_active_overlay(ctx: Option<String>) {
    portable::set_active_overlay(ctx);
}

/// Start the portable watchdog. Takes an `AppHandle` rather than an HWND
/// because the probe asks the main thread to run a closure; there is no window
/// handle involved.
#[cfg(not(windows))]
pub fn start(app: tauri::AppHandle) {
    portable::start(app);
}
