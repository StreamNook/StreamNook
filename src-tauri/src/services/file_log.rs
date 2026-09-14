//! Persistent app log: fans every `log` crate record out to the existing
//! env_logger (stderr, visible in dev runs) AND to an async file writer that
//! appends to `<app_data>/logs/streamnook.log`. Release builds run with
//! `windows_subsystem = "windows"` (no stderr), so without the file sink every
//! log line in a shipped build is written to a dead handle and lost.
//!
//! Write path copied from ll_diagnostics.rs: callers enqueue behind a
//! microseconds-held lock; one dedicated writer thread owns the only file
//! handle and drains on a short interval. A synchronous open-append-close per
//! line serializes the process behind Defender rescans of the growing file
//! (see the ll_diagnostics header for the field history).
//!
//! Level policy: a record reaches the file only if it passes BOTH the
//! env_logger filter (which carries module directives, e.g. silencing a noisy
//! third-party crate) and the Diagnostics-toggle level. With no RUST_LOG that
//! is Info when diagnostics are on and Warn when off; with RUST_LOG set, the
//! env filter wins for both sinks, so `RUST_LOG=streamnook=debug` does not
//! spray dependency debug into the file.

use log::{LevelFilter, Log, Metadata, Record};
use once_cell::sync::Lazy;
use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

const MAX_LOG_BYTES: u64 = 5_000_000;
const DRAIN_INTERVAL_MS: u64 = 300;
/// Hard cap on queued lines: a pathological flood drops lines instead of
/// ballooning memory while the writer is stalled.
const MAX_QUEUE_LINES: usize = 10_000;

/// File sink threshold, stored as `LevelFilter as usize` for a lock-free
/// hot-path gate. Applied on top of the env_logger filter.
static FILE_LEVEL: AtomicUsize = AtomicUsize::new(LevelFilter::Info as usize);
static QUEUE: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static WRITER: Once = Once::new();

/// Nothing reaches the file until the process proves it is the PRIMARY
/// instance. A warm deep link spawns a second process that lives just long
/// enough for the single-instance plugin to forward the URL and exit, and it
/// used to stamp its own "==== started ====" banner into the shared log,
/// corrupting boot-segment analysis (which anchors on the last banner). Only
/// the primary reaches the Tauri setup hook, which calls `arm()`; queued
/// early-boot lines then drain in order behind the banner. The writer thread
/// arms itself after a failsafe delay so a crash-before-setup still flushes
/// evidence (and a second instance alive that long is itself worth seeing).
static ARMED: AtomicBool = AtomicBool::new(false);
const ARM_FAILSAFE_SECS: u64 = 5;

pub fn arm() {
    ARMED.store(true, Ordering::Release);
}

pub struct FanoutLogger {
    stderr: env_logger::Logger,
}

impl Log for FanoutLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.stderr.enabled(metadata)
            || (metadata.level() as usize) <= FILE_LEVEL.load(Ordering::Relaxed)
    }

    fn log(&self, record: &Record) {
        if self.stderr.matches(record) {
            self.stderr.log(record);
        }
        // BOTH gates, always. The env_logger filter carries the module
        // directives (e.g. silencing a third-party crate), and FILE_LEVEL
        // carries the runtime Diagnostics toggle. Checking only the level let
        // module-filtered noise through to the file even though stderr had
        // dropped it, which is how fontdb's per-launch warnings kept landing
        // in logs people send us.
        let file_pass = self.stderr.matches(record)
            && (record.level() as usize) <= FILE_LEVEL.load(Ordering::Relaxed);
        if file_pass {
            enqueue(format_line(record));
        }
    }

    fn flush(&self) {
        self.stderr.flush();
    }
}

/// Install the fanout logger as the global logger and start the writer thread.
/// Single-shot (set_boxed_logger errors on a second call); called exactly once
/// from diagnostic_logger::init_logging.
pub fn install(stderr: env_logger::Logger, rust_log_set: bool) {
    let stderr_filter = stderr.filter();
    if rust_log_set {
        FILE_LEVEL.store(stderr_filter as usize, Ordering::Relaxed);
    }
    let logger = FanoutLogger { stderr };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(global_level_for(stderr_filter));
        ensure_writer();
    }
}

/// Diagnostics-toggle hook: sets the file threshold (shared with stderr when no
/// RUST_LOG, preserving the toggle's historical stderr behavior). No-op when
/// RUST_LOG is set (diagnostic_logger early-returns before calling, but guard
/// here too so the contract is local).
pub fn set_file_level(level: LevelFilter) {
    if std::env::var("RUST_LOG").is_ok() {
        return;
    }
    FILE_LEVEL.store(level as usize, Ordering::Relaxed);
}

/// The global max level the `log` crate must allow so both sinks see their
/// records: the louder of the stderr filter and the file threshold.
pub fn global_level() -> LevelFilter {
    let file = level_from_usize(FILE_LEVEL.load(Ordering::Relaxed));
    // Global install stores the stderr filter into FILE_LEVEL under RUST_LOG,
    // so file already reflects the env filter there; without RUST_LOG the
    // stderr filter tracks the toggle level too. max() keeps this honest if
    // those ever diverge.
    file
}

fn global_level_for(stderr_filter: LevelFilter) -> LevelFilter {
    stderr_filter.max(level_from_usize(FILE_LEVEL.load(Ordering::Relaxed)))
}

fn level_from_usize(v: usize) -> LevelFilter {
    match v {
        0 => LevelFilter::Off,
        1 => LevelFilter::Error,
        2 => LevelFilter::Warn,
        3 => LevelFilter::Info,
        4 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    }
}

/// `<app_data>/logs/streamnook.log`, creating the logs dir. Same resolver the
/// UI-hang watchdog and errors.log use, so every diagnostic lands in one place.
pub fn log_file_path() -> anyhow::Result<PathBuf> {
    let logs = crate::services::cache_service::get_app_data_dir()?.join("logs");
    std::fs::create_dir_all(&logs)?;
    Ok(logs.join("streamnook.log"))
}

fn enqueue(line: String) {
    if let Ok(mut q) = QUEUE.lock() {
        if q.len() < MAX_QUEUE_LINES {
            q.push(line);
        }
    }
}

fn format_line(record: &Record) -> String {
    format!(
        "{} {:<5} {} {}",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        record.level(),
        record.target(),
        scrub(&record.args().to_string())
    )
}

/// Redact credentials before a line ever reaches the queue. Masks the token
/// after any `oauth:` occurrence and blanks IRC PASS frames, covering the raw
/// server/auth lines irc_service logs at debug level.
fn scrub(s: &str) -> Cow<'_, str> {
    let has_oauth = s.contains("oauth:");
    let is_pass = s.trim_start().starts_with("PASS ");
    if !has_oauth && !is_pass {
        return Cow::Borrowed(s);
    }
    if is_pass {
        return Cow::Owned("PASS ***".to_string());
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("oauth:") {
        let after = i + "oauth:".len();
        out.push_str(&rest[..after]);
        out.push_str("***");
        let tail = &rest[after..];
        let skip = tail
            .find(char::is_whitespace)
            .unwrap_or(tail.len());
        rest = &tail[skip..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn ensure_writer() {
    WRITER.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("app-log-writer".into())
            .spawn(|| {
                let Ok(path) = log_file_path() else { return };
                let old_path = path.with_extension("log.old");
                let mut current: Option<(std::fs::File, u64)> = None;
                let mut banner_pending = true;
                let spawned = Instant::now();
                loop {
                    std::thread::sleep(Duration::from_millis(DRAIN_INTERVAL_MS));
                    if !ARMED.load(Ordering::Acquire) {
                        if spawned.elapsed() < Duration::from_secs(ARM_FAILSAFE_SECS) {
                            continue; // hold everything; a secondary instance exits first
                        }
                        ARMED.store(true, Ordering::Release);
                        enqueue("[FileLog] armed by failsafe (setup hook never ran)".into());
                    }
                    let mut batch = {
                        let mut q = match QUEUE.lock() {
                            Ok(q) => q,
                            Err(_) => continue,
                        };
                        std::mem::take(&mut *q)
                    };
                    if banner_pending {
                        batch.insert(
                            0,
                            format!(
                                "==== StreamNook {} started ====",
                                env!("CARGO_PKG_VERSION")
                            ),
                        );
                        banner_pending = false;
                    }
                    if batch.is_empty() {
                        continue;
                    }
                    if current.is_none() {
                        let f = OpenOptions::new().create(true).append(true).open(&path);
                        if let Ok(f) = f {
                            let written = f.metadata().map(|m| m.len()).unwrap_or(0);
                            current = Some((f, written));
                        }
                    }
                    // Rotate between drains, never mid-write: close the handle,
                    // remove the stale .old FIRST (Windows rename fails over an
                    // existing file), rename, reopen fresh.
                    if let Some((_, written)) = &current {
                        if *written > MAX_LOG_BYTES {
                            current = None;
                            let _ = std::fs::remove_file(&old_path);
                            let _ = std::fs::rename(&path, &old_path);
                            if let Ok(f) =
                                OpenOptions::new().create(true).append(true).open(&path)
                            {
                                current = Some((f, 0));
                            }
                        }
                    }
                    if let Some((f, written)) = current.as_mut() {
                        let mut out =
                            String::with_capacity(batch.iter().map(|l| l.len() + 1).sum());
                        for l in &batch {
                            out.push_str(l);
                            out.push('\n');
                        }
                        if f.write_all(out.as_bytes()).is_ok() {
                            *written += out.len() as u64;
                        }
                    }
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::scrub;

    #[test]
    fn scrub_masks_oauth_tokens_and_pass_frames() {
        assert_eq!(
            scrub("PASS oauth:abc123def"),
            "PASS ***"
        );
        assert_eq!(
            scrub("token is oauth:abc123def and more"),
            "token is oauth:*** and more"
        );
        assert_eq!(scrub("nothing secret here"), "nothing secret here");
    }
}
