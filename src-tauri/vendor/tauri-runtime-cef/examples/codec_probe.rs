//! Probes which video codecs the linked CEF build can actually decode.
//!
//! `canPlayType` is asked for the codec strings a Matrix client sees, and then
//! each sample is really loaded: an engine can answer `maybe` and still fail on
//! the first frame, so the probe waits for `loadedmetadata` or `error` rather
//! than trusting the advertisement.
//!
//! Point it at the two samples and run:
//!
//! ```text
//! PROBE_MP4=/tmp/sample.mp4 PROBE_WEBM=/tmp/sample.webm \
//!   cargo run --example codec_probe
//! ```
//!
//! Prints `PROBE-RESULT {...}` on stdout and exits 0. A 60s watchdog exits 2.

use std::{borrow::Cow, env, fs};

const PAGE: &str = r#"<!doctype html><title>codec probe</title><script>
const TYPES = {
  mp4H264Aac: 'video/mp4; codecs="avc1.42E01E, mp4a.40.2"',
  mp4Bare: 'video/mp4',
  webmVp9Opus: 'video/webm; codecs="vp9, opus"',
  webmVp8: 'video/webm; codecs="vp8, vorbis"',
};

/* Through a blob URL, matching how the client publishes media: a media element
   range-requests a custom scheme, and a handler that answers 200-with-the-lot
   fails the load for reasons that have nothing to do with the codec. */
async function play(path, type) {
  const bytes = await (await fetch(path)).arrayBuffer();
  const url = URL.createObjectURL(new Blob([bytes], { type }));
  return new Promise((resolve) => {
    const video = document.createElement('video');
    let frames = 0;
    const done = (verdict) => { URL.revokeObjectURL(url); video.remove(); resolve(verdict); };
    video.addEventListener('ended', () =>
      done('plays ' + video.videoWidth + 'x' + video.videoHeight + ' frames=' + frames));
    video.addEventListener('error', () =>
      done('error: ' + (video.error ? video.error.code + ' ' + video.error.message : 'unknown')));
    setTimeout(() => done('timeout frames=' + frames + ' t=' + video.currentTime), 30000);
    const tick = () => { frames += 1; video.requestVideoFrameCallback(tick); };
    video.requestVideoFrameCallback(tick);
    video.muted = true;
    document.body.appendChild(video);
    video.src = url;
    video.play().catch((e) => done('play rejected: ' + e));
  });
}

(async () => {
  const r = { canPlayType: {} };
  const probe = document.createElement('video');
  for (const [name, type] of Object.entries(TYPES)) {
    // '' is the spec's no; 'maybe'/'probably' are both a yes.
    r.canPlayType[name] = probe.canPlayType(type) || '(empty)';
  }
  r.mp4Playback = await play('/sample.mp4', 'video/mp4');
  r.webmPlayback = await play('/sample.webm', 'video/webm');
  await fetch('/report', { method: 'POST', body: JSON.stringify(r, null, 1) });
})();
</script>"#;

fn sample(var: &str) -> Vec<u8> {
  let path = env::var(var).unwrap_or_else(|_| panic!("{var} must point at a sample file"));
  fs::read(&path).unwrap_or_else(|err| panic!("reading {path}: {err}"))
}

fn main() {
  let mp4 = sample("PROBE_MP4");
  let webm = sample("PROBE_WEBM");

  tauri_runtime_cef::configure(tauri_runtime_cef::CefConfig {
    identifier: "cef-codec-probe".into(),
    command_line_args: {
      let mut args = vec![
        ("--use-mock-keychain".into(), None),
        ("password-store".into(), Some("basic".into())),
      ];
      // `;`-separated, so a value may carry the commas a feature list needs:
      //   PROBE_SWITCHES='disable-features=A,B;use-angle=vulkan'
      if let Ok(extra) = env::var("PROBE_SWITCHES") {
        for switch in extra.split(';').map(str::trim).filter(|s| !s.is_empty()) {
          match switch.split_once('=') {
            Some((key, value)) => args.push((key.to_owned(), Some(value.to_owned()))),
            None => args.push((switch.to_owned(), None)),
          }
        }
      }
      args
    },
    custom_schemes: vec!["tauri".into(), "ipc".into(), "asset".into(), "probe".into()],
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
    .register_uri_scheme_protocol("probe", move |_ctx, request| {
      let respond = |mime: &str, body: Cow<'static, [u8]>| {
        tauri::http::Response::builder()
          .header("content-type", mime)
          .body(body)
          .unwrap()
      };
      match request.uri().path() {
        "/" => respond("text/html", Cow::Borrowed(PAGE.as_bytes())),
        "/sample.mp4" => respond("video/mp4", Cow::Owned(mp4.clone())),
        "/sample.webm" => respond("video/webm", Cow::Owned(webm.clone())),
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
