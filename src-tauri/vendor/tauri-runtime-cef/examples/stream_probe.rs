//! Live proof that `register_streaming_scheme_handler` streams incrementally
//! under CEF — the counterpart to `sw_probe`, for the streaming path.
//!
//! The page at `probe://app/` opens an `EventSource` on `/sse`. The handler
//! writes one `data:` tick every 150ms and NEVER finishes the response. A
//! buffered handler could deliver nothing until the whole body completed, so
//! the page receiving three ticks spread over ~300ms proves the bytes arrive
//! chunk-by-chunk. The page POSTs the result to `/report`, which prints
//! `PROBE-RESULT {...}` and exits 0. A 60s watchdog exits 2.
//!
//! Headless run (this box has no GPU and cannot fork the sandbox zygote or a
//! renderer subprocess — subprocess mode silently crash-loops the navigation,
//! so `--single-process` is required here):
//!   cargo build --example stream_probe
//!   cp target/debug/examples/stream_probe target/debug/
//!   cd target/debug && DISPLAY=:99 LD_LIBRARY_PATH=. ./stream_probe \
//!     --no-sandbox --single-process --disable-gpu --use-gl=angle \
//!     --use-angle=swiftshader --enable-unsafe-swiftshader
//! Expected: PROBE-RESULT {"secureContext":true,"streamed":true,"ticks":3,...,"incremental":true}

use std::borrow::Cow;
use std::time::Duration;

const PAGE: &str = r#"<!doctype html><title>stream probe</title><script>
(async () => {
  const r = { secureContext: window.isSecureContext };
  const arrivals = [];
  try {
    await new Promise((resolve, reject) => {
      const es = new EventSource('/sse');
      const timer = setTimeout(() => { es.close(); reject(new Error('sse timeout')); }, 8000);
      es.onmessage = (e) => {
        arrivals.push({ v: e.data, t: performance.now() });
        if (arrivals.length >= 3) { clearTimeout(timer); es.close(); resolve(); }
      };
      es.onerror = () => {}; // EventSource retries on its own; keep waiting.
    });
    r.streamed = true;
    r.ticks = arrivals.length;
    r.values = arrivals.map((a) => a.v);
    r.spanMs = Math.round(arrivals[arrivals.length - 1].t - arrivals[0].t);
    // Ticks are paced 150ms apart, so a real stream spans >=100ms; a single
    // buffered blob would arrive all at once (span ~0).
    r.incremental = r.spanMs >= 100;
  } catch (e) {
    r.streamed = false;
    r.error = String(e);
  }
  await fetch('/report', { method: 'POST', body: JSON.stringify(r) });
})();
</script>"#;

fn main() {
  tauri_runtime_cef::configure(tauri_runtime_cef::CefConfig {
    identifier: "cef-stream-probe".into(),
    command_line_args: vec![
      ("--use-mock-keychain".into(), None),
      ("password-store".into(), Some("basic".into())),
    ],
    custom_schemes: vec!["tauri".into(), "ipc".into(), "asset".into(), "probe".into()],
    cookieable_schemes: vec!["probe".into()],
    ..Default::default()
  });

  if std::env::args().any(|arg| arg.starts_with("--type=")) {
    tauri_runtime_cef::run_cef_helper_process();
    return;
  }

  std::thread::spawn(|| {
    std::thread::sleep(Duration::from_secs(60));
    println!("PROBE-RESULT {{\"timeout\":true}}");
    std::process::exit(2);
  });

  // The streaming handler owns every probe:// request (process_request consults
  // the streaming registry before the buffered path). The buffered registration
  // below still has to exist so the runtime installs the scheme factory +
  // per-webview registry entry; its handler is never actually called.
  tauri_runtime_cef::register_streaming_scheme_handler(
    "probe",
    Box::new(|_label, request, responder| match request.uri().path() {
      "/" => {
        let head = http::Response::builder()
          .header("content-type", "text/html")
          .body(())
          .unwrap();
        let mut writer = responder.respond(head);
        let _ = writer.write(PAGE.as_bytes().to_vec());
      }
      "/sse" => {
        let head = http::Response::builder()
          .header("content-type", "text/event-stream")
          .header("cache-control", "no-cache")
          .body(())
          .unwrap();
        let mut writer = responder.respond(head);
        for i in 0..12 {
          if writer.write(format!("data: {i}\n\n").into_bytes()).is_err() {
            break; // renderer cancelled — the page got its ticks and closed.
          }
          std::thread::sleep(Duration::from_millis(150));
        }
      }
      "/report" => {
        println!("PROBE-RESULT {}", String::from_utf8_lossy(request.body()));
        std::process::exit(0);
      }
      _ => {
        let head = http::Response::builder().status(404).body(()).unwrap();
        responder.respond(head);
      }
    }),
  );

  type Rt = tauri_runtime_cef::CefRuntime<tauri::EventLoopMessage>;
  tauri::Builder::<Rt>::new()
    .register_uri_scheme_protocol("probe", |_ctx, _request| {
      // Unreachable: the streaming handler owns every probe:// request.
      tauri::http::Response::builder()
        .status(500)
        .body(Cow::Borrowed(
          &b"buffered probe handler should be unreachable"[..],
        ))
        .unwrap()
    })
    .setup(|app| {
      tauri::WebviewWindowBuilder::new(
        app,
        "probe",
        tauri::WebviewUrl::External("probe://app/".parse().unwrap()),
      )
      .build()?;
      Ok(())
    })
    .run(tauri::test::mock_context(tauri::test::noop_assets()))
    .expect("probe app run");
}
