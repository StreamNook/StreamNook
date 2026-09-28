//! Probes Chromium platform behavior for a custom scheme registered
//! standard+secure+CORS+fetch: secure context, service worker registration,
//! document.cookie, localStorage, CacheStorage.
//!
//! The page served at `probe://app/` runs the checks and POSTs a JSON blob
//! back to `/report`; the handler prints it as `PROBE-RESULT {...}` on stdout
//! and exits 0. A 60s watchdog exits 2.

use std::borrow::Cow;

const PAGE: &str = r#"<!doctype html><title>probe</title><script>
(async () => {
  const r = { secureContext: window.isSecureContext,
              swInNavigator: 'serviceWorker' in navigator,
              cachesInWindow: 'caches' in window };
  try { document.cookie = 'probe=1; path=/';
        r.cookie = document.cookie.includes('probe=1'); }
  catch (e) { r.cookie = 'throw: ' + e; }
  try { localStorage.setItem('probe', '1');
        r.localStorage = localStorage.getItem('probe') === '1'; }
  catch (e) { r.localStorage = 'throw: ' + e; }
  if (r.swInNavigator) {
    try {
      const reg = await navigator.serviceWorker.register('/sw.js');
      await navigator.serviceWorker.ready;
      r.sw = 'registered';
      r.swScope = reg.scope;
    } catch (e) { r.sw = 'error'; r.swError = String(e); }
  }
  await fetch('/report', { method: 'POST', body: JSON.stringify(r) });
})();
</script>"#;

const SW: &str = r#"self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(clients.claim()));
self.addEventListener('fetch', () => {});"#;

fn main() {
  tauri_runtime_cef::configure(tauri_runtime_cef::CefConfig {
    identifier: "cef-sw-probe".into(),
    // On a headless box (Xvfb, no GPU, possibly no unprivileged-userns) the
    // CEF sandbox zygote and/or GPU process can fail to launch. If this
    // example traps at boot, build it `--no-default-features` (drops the
    // `sandbox` feature so `no_sandbox` is honored) and add the switches
    // `--no-sandbox --disable-gpu --in-process-gpu` here. That combination is
    // box-dependent and flaky; a real desktop keeps the sandbox default and
    // needs none of them.
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
    std::thread::sleep(std::time::Duration::from_secs(60));
    println!("PROBE-RESULT {{\"timeout\":true}}");
    std::process::exit(2);
  });

  type Rt = tauri_runtime_cef::CefRuntime<tauri::EventLoopMessage>;
  tauri::Builder::<Rt>::new()
    .register_uri_scheme_protocol("probe", |_ctx, request| {
      let respond = |mime: &str, body: &'static str| {
        tauri::http::Response::builder()
          .header("content-type", mime)
          .body(Cow::Borrowed(body.as_bytes()))
          .unwrap()
      };
      match request.uri().path() {
        "/" => respond("text/html", PAGE),
        "/sw.js" => respond("text/javascript", SW),
        "/report" => {
          println!("PROBE-RESULT {}", String::from_utf8_lossy(request.body()));
          std::process::exit(0);
        }
        _ => tauri::http::Response::builder()
          .status(404)
          .body(Cow::Borrowed(&b""[..]))
          .unwrap(),
      }
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
