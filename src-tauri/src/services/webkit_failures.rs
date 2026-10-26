//! Records a macOS WebKit web content process that died, the macOS twin of
//! `webview2_failures`.
//!
//! WebKit runs each page in its own WebContent process. When that process
//! crashes or is killed (macOS ends one that grows past its memory limit), the
//! page goes blank and every click into it does nothing until it is loaded
//! again. Tauri's default handler reloaded it silently, so a viewer saw the app
//! "freeze" or "refresh" and the log said nothing. This records which window
//! lost its page and how often, synchronously (`file_log::record_now`, the same
//! record the Windows renderer failures write), then reloads it, as Windows
//! does for a renderer that exited.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Terminations of one window inside this span are counted together, so a
/// page that keeps dying (memory) reads as a loop in the log.
const SPAN: Duration = Duration::from_secs(5 * 60);

static RECENT: Mutex<Option<HashMap<String, Vec<Instant>>>> = Mutex::new(None);

/// How many terminations `label` has had inside `SPAN`, this one included.
fn count(label: &str, now: Instant) -> usize {
    let mut guard = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    let times = guard.get_or_insert_with(HashMap::new).entry(label.to_string()).or_default();
    times.retain(|t| now.duration_since(*t) <= SPAN);
    times.push(now);
    times.len()
}

/// The hook for `Builder::on_web_content_process_terminate` (main thread).
pub fn content_process_terminated(webview: &crate::rt::Webview) {
    let label = webview.label().to_string();
    let n = count(&label, Instant::now());
    let summary = format!(
        "WebKit web content process ended in window '{label}' ({n} in the last {} min); the page went blank",
        SPAN.as_secs() / 60
    );
    let recovery = match webview.reload() {
        Ok(()) => "reloading the page".to_string(),
        Err(e) => format!("reload failed: {e}"),
    };
    crate::services::file_log::record_now("WebKit", &summary, "Recovery", &recovery);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminations_are_counted_per_window_inside_the_span() {
        let t = Instant::now();
        assert_eq!(count("test-a", t), 1);
        assert_eq!(count("test-a", t + Duration::from_secs(10)), 2);
        assert_eq!(count("test-b", t + Duration::from_secs(11)), 1);
        assert_eq!(count("test-a", t + SPAN * 2), 1, "old ones age out");
    }
}
