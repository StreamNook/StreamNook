//! Records WebView2 process failures, the crashes that take the page (or every
//! page) down without the Rust process noticing.
//!
//! A renderer that dies leaves the window showing a blank or "page crashed"
//! surface, and a browser process that dies leaves every webview on it dead:
//! from then on each call into them fails with 0x8007139F (ERROR_INVALID_STATE),
//! which tauri-runtime-wry logs as a bare "WebView2 error" line. To the viewer
//! either one reads as "the app crashed", and before this nothing recorded
//! which process went, why, or with what exit code.
//!
//! The record is written synchronously (`file_log::record_now`) because the
//! next thing that often happens is the viewer closing the dead window.

/// Watch `win`'s WebView2 for process failures. Recovers the one case that
/// can be recovered in place: a main-frame renderer that exited is reloaded,
/// which brings the app back instead of leaving a dead page up.
#[cfg(windows)]
pub fn watch(win: &crate::rt::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2ProcessFailedEventArgs,
        ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_PROCESS_FAILED_KIND,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_REASON,
    };
    use webview2_com::ProcessFailedEventHandler;
    use windows::core::Interface;

    let label = win.label().to_string();
    let dispatched = win.with_webview(move |platform_webview| unsafe {
        let core = match platform_webview.controller().CoreWebView2() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[WebView2] no CoreWebView2 to watch for process failures on '{label}': {e}");
                return;
            }
        };
        let window = label.clone();
        let handler = ProcessFailedEventHandler::create(Box::new(
            move |sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2ProcessFailedEventArgs>| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                let _ = args.ProcessFailedKind(&mut kind);
                let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                let mut exit_code: i32 = 0;
                if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                    let _ = args2.Reason(&mut reason);
                    let _ = args2.ExitCode(&mut exit_code);
                }
                let summary = format!(
                    "WebView2 {} failed in window '{}' (reason {}, exit code {:#010x})",
                    kind_name(kind.0),
                    window,
                    reason_name(reason.0),
                    exit_code as u32
                );
                let recovering = kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
                crate::services::file_log::record_now(
                    "WebView2",
                    &summary,
                    "Recovery",
                    if recovering {
                        "reloading the page"
                    } else {
                        "none in place"
                    },
                );
                if recovering {
                    if let Some(sender) = sender {
                        let _ = sender.Reload();
                    }
                }
                Ok(())
            },
        ));
        // webview2-com 0.38 types the registration token as a plain i64.
        let mut token: i64 = 0;
        if let Err(e) = core.add_ProcessFailed(&handler, &mut token) {
            log::warn!("[WebView2] add_ProcessFailed failed on '{label}': {e}");
        }
    });
    if let Err(e) = dispatched {
        log::warn!("[WebView2] couldn't reach the webview to watch for process failures: {e}");
    }
}

/// `COREWEBVIEW2_PROCESS_FAILED_KIND` by name.
fn kind_name(kind: i32) -> String {
    match kind {
        0 => "browser process".into(),
        1 => "renderer process".into(),
        2 => "renderer process (unresponsive)".into(),
        3 => "frame renderer process".into(),
        4 => "utility process".into(),
        5 => "sandbox helper process".into(),
        6 => "GPU process".into(),
        7 => "PPAPI plugin process".into(),
        8 => "PPAPI broker process".into(),
        9 => "unknown process".into(),
        other => format!("process kind {other}"),
    }
}

/// `COREWEBVIEW2_PROCESS_FAILED_REASON` by name.
fn reason_name(reason: i32) -> String {
    match reason {
        0 => "unexpected".into(),
        1 => "unresponsive".into(),
        2 => "terminated".into(),
        3 => "crashed".into(),
        4 => "launch failed".into(),
        5 => "out of memory".into(),
        6 => "profile deleted".into(),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_kinds_and_reasons_read_as_words() {
        assert_eq!(kind_name(0), "browser process");
        assert_eq!(kind_name(6), "GPU process");
        assert_eq!(kind_name(42), "process kind 42");
        assert_eq!(reason_name(3), "crashed");
        assert_eq!(reason_name(9), "9");
    }
}
