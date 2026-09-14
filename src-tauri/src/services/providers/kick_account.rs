//! Kick account sync: import the user's real follows and subscriptions.
//!
//! Kick's OFFICIAL API (api.kick.com) has no "channels this user follows"
//! endpoint — verified against their docs. Their website's own API does:
//! `GET kick.com/api/v2/channels/followed` and `/api/v2/user/subscriptions`,
//! both of which answer 401 rather than 404 when signed out, so they exist and
//! simply want the site session.
//!
//! Getting at them needs a kick.com login, not the OAuth connection: OAuth
//! tokens address api.kick.com, these endpoints address the website. Rather
//! than harvesting cookies and replaying them (kick.com/api is Cloudflare-gated
//! and rejects a plain client's TLS fingerprint regardless of cookies), we run
//! the request from PAGE CONTEXT inside the same persistent WebView2 profile
//! the playback resolver already uses — same origin, cookies attached,
//! Cloudflare already cleared.
//!
//! Deliberate split of concerns: this undocumented path runs RARELY (on connect
//! and on an explicit sync) purely to obtain the LIST. Live status for those
//! channels then comes from the official, sanctioned API on the regular poll.
//! So the fragile part is never on the hot path, and if Kick ever changes it the
//! imported list still works.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use tokio::sync::{oneshot, Mutex};
use tokio::time::{timeout, Duration};

use crate::services::providers::app_handle;

/// One channel the user follows on Kick, as reported by the import webview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickFollowedChannel {
    pub slug: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub profile_pic: Option<String>,
    #[serde(default)]
    pub is_live: bool,
    /// True when this channel also appears in the user's subscriptions.
    #[serde(default)]
    pub subscribed: bool,
}

/// What the injected script reports back.
#[derive(Debug, Clone, Deserialize)]
pub struct KickImportReport {
    #[serde(default)]
    pub channels: Vec<KickFollowedChannel>,
    /// "ok" | "unauthenticated" | "error"
    pub status: String,
    /// Top-level JSON keys of the first page, logged once so an unexpected
    /// response shape is diagnosable without guessing.
    #[serde(default)]
    pub shape: Option<String>,
}

static PENDING: OnceLock<Mutex<HashMap<String, oneshot::Sender<KickImportReport>>>> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);

fn pending() -> &'static Mutex<HashMap<String, oneshot::Sender<KickImportReport>>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called by the `report_kick_follows` command when the injected script finishes.
pub async fn resolve_pending(label: &str, report: KickImportReport) {
    if let Some(tx) = pending().lock().await.remove(label) {
        let _ = tx.send(report);
    }
}

/// Whether a kick.com website session exists in the profile. Cheap probe: run
/// the import in a hidden window and see whether it comes back authenticated.
pub async fn is_connected() -> bool {
    matches!(import(false).await, Ok(report) if report.status == "ok")
}

/// Import the user's follows + subscriptions.
///
/// `interactive` opens a VISIBLE window at kick.com/login so the user can sign
/// in; the injected script polls until the session works, then reports and the
/// window closes itself. Non-interactive runs hidden and fails fast when there
/// is no session yet.
pub async fn import(interactive: bool) -> Result<KickImportReport> {
    #[cfg(not(windows))]
    {
        let _ = interactive;
        return Err(anyhow!("Kick account sync is only wired for Windows so far"));
    }
    #[cfg(windows)]
    {
        use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

        let app = app_handle().ok_or_else(|| anyhow!("app handle not available for Kick sync"))?;
        let label = format!("kick-sync-{}", SEQ.fetch_add(1, Ordering::Relaxed));

        let (tx, rx) = oneshot::channel::<KickImportReport>();
        pending().lock().await.insert(label.clone(), tx);

        // A profile of its OWN, deliberately not the resolver's: signing in runs
        // the gauntlet of Kick's bot defense, and a wedged challenge state must
        // never be able to take playback down with it.
        let profile = crate::services::providers::kick::account_profile_dir(&app);
        let script = import_script(&label, interactive);
        let start_url = if interactive {
            "https://kick.com/login"
        } else {
            "https://kick.com/"
        };
        let parsed = start_url.parse().map_err(|e| anyhow!("bad url: {}", e))?;

        // A stale window from a previous attempt would hold the profile lock.
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.destroy();
        }

        let mut builder = WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::External(parsed))
            .data_directory(profile)
            .initialization_script(&script);
        builder = if interactive {
            builder.title("Sign in to Kick").inner_size(520.0, 720.0)
        } else {
            builder
                .visible(false)
                .skip_taskbar(true)
                .focused(false)
                .inner_size(1.0, 1.0)
        };
        let win = builder
            .build()
            .map_err(|e| anyhow!("Kick sync window failed: {}", e))?;

        // Interactive gets a long budget (the user is typing a password, maybe
        // doing 2FA); the silent probe should not hang the caller.
        let budget = if interactive {
            Duration::from_secs(300)
        } else {
            Duration::from_secs(25)
        };
        let result = timeout(budget, rx).await;
        pending().lock().await.remove(&label);
        let _ = win.destroy();

        match result {
            Ok(Ok(report)) => {
                if let Some(shape) = &report.shape {
                    log::debug!("[Kick] followed-channels payload keys: {}", shape);
                }
                log::info!(
                    "[Kick] account sync {}: {} channel(s)",
                    report.status,
                    report.channels.len()
                );
                Ok(report)
            }
            Ok(Err(_)) => Err(anyhow!("Kick sync window closed before reporting")),
            Err(_) if interactive => Err(anyhow!("Kick sign-in timed out")),
            Err(_) => Err(anyhow!("not signed in to kick.com")),
        }
    }
}

/// The injected script. Runs at document-start in the sync window, polls the
/// followed-channels endpoint until it authenticates (interactive) or once
/// (silent), pages through the results, folds in subscriptions, and reports.
fn import_script(label: &str, interactive: bool) -> String {
    let js_label = serde_json::to_string(label).unwrap_or_else(|_| "\"\"".to_string());
    // How long to WAIT for the user to finish signing in. This is a local cookie
    // watch rather than a request loop, so a generous window costs nothing.
    let wait_seconds = if interactive { 300 } else { 0 };
    format!(
        r#"(function() {{
  var label = {js_label};
  var reported = false;
  var waited = 0;
  var tries = 0;
  var WAIT_SECONDS = {wait_seconds};

  function report(status, channels, shape) {{
    if (reported) return;
    reported = true;
    window.__TAURI_INTERNALS__.invoke('report_kick_follows', {{
      label: label,
      report: {{ status: status, channels: channels || [], shape: shape || null }}
    }});
  }}

  // `session_token` and `XSRF-TOKEN` are NOT HttpOnly on kick.com, so page
  // context can read them. Re-read per call: Kick's Laravel session regenerates
  // on login and the token rotates, so a cached value silently 401s.
  function cookies() {{
    var out = {{}};
    document.cookie.split(';').forEach(function(part) {{
      var i = part.indexOf('=');
      if (i < 0) return;
      var k = part.slice(0, i).trim();
      // Sanctum tokens contain '|', which arrives percent-encoded. Decoding is
      // required — sending the raw value is a silent 401.
      try {{ out[k] = decodeURIComponent(part.slice(i + 1)); }} catch (e) {{ out[k] = part.slice(i + 1); }}
    }});
    return out;
  }}

  // Breadcrumbs to the app log. Sign-in runs in a window the user is looking at
  // but whose console we never see, so without these a failed import is silent.
  function diag(note) {{
    try {{ window.__TAURI_INTERNALS__.invoke('report_kick_resolve_diag', {{ label: label, note: String(note) }}); }} catch (e) {{}}
  }}

  function getJson(url) {{
    var c = cookies();
    var headers = {{ 'Accept': 'application/json' }};
    // Cookie auth alone works from a same-origin page context (Laravel Sanctum
    // promotes the session on a stateful domain), but every shipped client that
    // reads this endpoint also sends the bearer, so send both and remove the
    // ambiguity. X-XSRF-TOKEN is only needed for mutations; harmless on a GET.
    if (c['session_token']) headers['Authorization'] = 'Bearer ' + c['session_token'];
    if (c['XSRF-TOKEN']) headers['X-XSRF-TOKEN'] = c['XSRF-TOKEN'];
    return fetch(url, {{ credentials: 'include', headers: headers, cache: 'no-store' }})
      .then(function(r) {{ return r.json().catch(function() {{ return null; }}).then(function(j) {{ return {{ status: r.status, body: j }}; }}); }});
  }}

  // The payload shape is undocumented, so read it tolerantly: accept a bare
  // array, {{data:[...]}} or {{channels:[...]}}, and pull each field from any of
  // the places Kick's API has historically put it.
  function rows(body) {{
    if (Array.isArray(body)) return body;
    if (!body) return [];
    if (Array.isArray(body.data)) return body.data;
    if (Array.isArray(body.channels)) return body.channels;
    if (body.data && Array.isArray(body.data.channels)) return body.data.channels;
    return [];
  }}

  // The live shape is
  //   {{ channel_slug, user_username, profile_picture, is_live, viewer_count,
  //      category_name, session_title }}
  // but the subscriptions endpoint nests differently, so read tolerantly.
  function slugOf(e) {{
    return (e && (e.channel_slug
      || e.slug
      || (e.channel && e.channel.slug)
      || (e.user && e.user.username && String(e.user.username).toLowerCase())
      || (e.channel && e.channel.user && e.channel.user.username && String(e.channel.user.username).toLowerCase()))) || null;
  }}
  function nameOf(e) {{
    return (e && (e.user_username
      || (e.user && e.user.username)
      || e.username
      || (e.channel && e.channel.user && e.channel.user.username)
      || e.name)) || null;
  }}
  function picOf(e) {{
    return (e && ((e.user && e.user.profile_pic)
      || e.profile_pic
      || (e.channel && e.channel.user && e.channel.user.profile_pic)
      || e.profile_picture)) || null;
  }}
  function liveOf(e) {{
    if (!e) return false;
    if (e.is_live === true) return true;
    if (e.livestream) return true;
    if (e.channel && e.channel.livestream) return true;
    if (e.stream && e.stream.is_live === true) return true;
    return false;
  }}

  // Cursor pagination: the response carries `nextCursor`, and a null/absent one
  // ends it. The page cap is a defensive stop so a cursor that never terminates
  // can't spin forever.
  function collect(base, cursor, acc, shape, page) {{
    var sep = base.indexOf('?') >= 0 ? '&' : '?';
    return getJson(base + sep + 'cursor=' + cursor + '&_cb=' + Date.now()).then(function(res) {{
      if (res.status === 401 || res.status === 403) return {{ unauth: true, items: acc, shape: shape }};
      if (res.status !== 200) diag(base + ' returned ' + res.status);
      var list = rows(res.body);
      if (page === 0 && list.length === 0 && res.body) {{
        // 200 but nothing parsed: the payload shape moved. Log its top-level
        // keys so the mismatch is visible instead of looking like "no follows".
        diag('no rows parsed from ' + base + '; keys: ' + (typeof res.body === 'object' ? Object.keys(res.body).join(',') : typeof res.body));
      }}
      if (!shape && res.body && typeof res.body === 'object' && !Array.isArray(res.body)) {{
        shape = Object.keys(res.body).join(',');
      }}
      acc = acc.concat(list);
      var next = res.body && res.body.nextCursor;
      if (list.length === 0 || next === null || next === undefined || page >= 20) {{
        return {{ unauth: false, items: acc, shape: shape }};
      }}
      return collect(base, next, acc, shape, page + 1);
    }}).catch(function() {{ return {{ unauth: false, items: acc, shape: shape }}; }});
  }}

  // Wait for the sign-in by watching document.cookie, which costs no requests at
  // all. The first version re-called the API every 2s while the user typed — up
  // to 150 requests per attempt against endpoints Kick rate-limits at the
  // Cloudflare edge, which is enough to get the whole IP throttled and produce
  // the body-less 429 that the site reports as "an unknown error".
  // Deliberately does NOT require `session_token` to be visible: whether that
  // cookie is readable from page context is Kick's choice, not ours, and gating
  // on it means a completed login can sit here forever. Any of three signals
  // starts an attempt — the cookie appearing, the page leaving /login, or a slow
  // safety tick — and the attempt itself is the real test. Ticks are 5s apart and
  // capped, so this stays far below the request volume that got the IP
  // rate-limited earlier.
  function waitForSession() {{
    var c = cookies();
    if (waited === 0) {{
      // Names only, never values: tells us whether the session cookie is even
      // visible to page script, without putting a credential in a log.
      diag('cookies visible at start: ' + (Object.keys(c).join(',') || '(none)'));
    }}
    var hasCookie = !!c['session_token'];
    var leftLogin = location.pathname.indexOf('/login') < 0;
    if (hasCookie || leftLogin || (waited > 0 && waited % 5 === 0)) {{
      diag('attempting (session_token=' + hasCookie + ', path=' + location.pathname + ')');
      run();
      return;
    }}
    if (waited >= WAIT_SECONDS) {{ report('unauthenticated', [], null); return; }}
    waited++;
    setTimeout(waitForSession, 1000);
  }}

  function run() {{
    tries++;
    collect('https://kick.com/api/v2/channels/followed', 0, [], null, 0).then(function(followed) {{
      if (followed.unauth) {{
        // Not signed in YET — the user may still be mid-login. Fall back into
        // the waiting loop rather than ending the flow; it only gives up once
        // the whole window expires.
        diag('attempt ' + tries + ': not authenticated yet');
        if (waited >= WAIT_SECONDS) {{ report('unauthenticated', [], null); return; }}
        waited += 3;
        setTimeout(waitForSession, 3000);
        return;
      }}
      diag('authenticated; followed rows: ' + followed.items.length);
      // Subscriptions are a bonus: never let their failure lose the follow list.
      // v2 first — Kick bot-walls several /api/v1 routes with a 403 "security
      // policy", so v1 is only a fallback.
      collect('https://kick.com/api/v2/user/subscriptions', 0, [], null, 0).then(function(subs) {{
        return subs.unauth || subs.items.length === 0
          ? collect('https://kick.com/api/v1/subscriptions', 0, [], null, 0)
          : subs;
      }}).then(function(subs) {{
        var subSlugs = {{}};
        subs.items.forEach(function(s) {{ var sl = slugOf(s); if (sl) subSlugs[String(sl).toLowerCase()] = true; }});
        var seen = {{}};
        var out = [];
        followed.items.forEach(function(e) {{
          var slug = slugOf(e);
          if (!slug) return;
          slug = String(slug).toLowerCase();
          if (seen[slug]) return;
          seen[slug] = true;
          out.push({{
            slug: slug,
            username: nameOf(e),
            profile_pic: picOf(e),
            is_live: liveOf(e),
            subscribed: !!subSlugs[slug]
          }});
        }});
        // A subscribed channel the user doesn't formally follow still belongs
        // in the list — they clearly care about it.
        subs.items.forEach(function(s) {{
          var slug = slugOf(s);
          if (!slug) return;
          slug = String(slug).toLowerCase();
          if (seen[slug]) return;
          seen[slug] = true;
          out.push({{ slug: slug, username: nameOf(s), profile_pic: picOf(s), is_live: liveOf(s), subscribed: true }});
        }});
        report('ok', out, followed.shape);
      }});
    }});
  }}

  // This script is injected into every page the window loads, which during a
  // combined sign-in includes id.kick.com's consent page. Only kick.com itself
  // is same-origin with the followed-channels endpoint, so stay dormant anywhere
  // else and let the navigation to kick.com start the real work.
  if (location.hostname !== 'kick.com' && location.hostname !== 'www.kick.com') return;

  // fetch() needs only the origin + cookies, not a painted DOM. Interactive
  // waits for the session; the silent probe just asks once.
  if (WAIT_SECONDS > 0) waitForSession(); else run();
}})();"#
    )
}

/// Sign in to Kick once, in ONE window, and come away with both credentials.
///
/// Kick needs two things that look like one to the user: an OAuth token (chat,
/// moderation) and a kick.com site session (the follow list, which their public
/// API does not expose). Doing those as two prompts — a system-browser consent
/// and then an in-app login — is the thing that felt broken.
///
/// It runs in the app's shared login OVERLAY — the same in-app surface Twitch
/// uses — rather than a popup window, so signing into Kick looks like signing
/// into Twitch. The overlay is told which web profile to use, and everything
/// after that is driven from here by window label.
///
/// The order matters and is the part worth not rearranging. The SITE login runs
/// first, because consent at id.kick.com does not by itself leave apex kick.com
/// cookies — leading with consent left the flow waiting on a session that could
/// never appear. Once the site session exists, the follow list is readable and
/// the consent leg below is usually a single click.
#[cfg(windows)]
pub async fn sign_in() -> Result<KickImportReport> {
    use tauri::Manager;

    const LABEL: &str = "kick-login";
    let app = app_handle().ok_or_else(|| anyhow!("app handle not available for Kick sign-in"))?;

    // Bind the loopback and build the consent URL up front, so the redirect can
    // never arrive before something is listening for it.
    let (auth_url, auth_pending) = crate::services::kick_auth_service::begin_auth().await?;

    // Hand the overlay the login page. React measures the app body and mounts the
    // webview at that rect; `kick-account` selects Kick's own cookie jar.
    crate::commands::twitch::emit_overlay_open_with(
        &app,
        LABEL,
        "https://kick.com/login",
        "fullbody",
        Some("kick-account"),
    )
    .map_err(|e| anyhow!("couldn't open the Kick sign-in overlay: {}", e))?;

    // The overlay mounts asynchronously (Rust asks, React measures, Rust builds),
    // so wait for the window to exist before addressing it.
    let mut win = None;
    for _ in 0..60 {
        if let Some(w) = app.get_webview_window(LABEL) {
            win = Some(w);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let win = win.ok_or_else(|| {
        crate::commands::twitch::dismiss_login_overlay(&app, LABEL);
        anyhow!("Kick sign-in overlay never mounted")
    })?;

    // Leg 1: wait for the site session by polling the webview's cookie jar. That
    // jar sees HttpOnly cookies, which page script cannot, so it is the only
    // reliable signal that the user is actually signed in.
    let mut report: Option<KickImportReport> = None;
    for _ in 0..150 {
        if app.get_webview_window(LABEL).is_none() {
            log::info!("[Kick] sign-in overlay dismissed by the user");
            // The child webview is gone, but the React overlay frame and the
            // ui_hang_watchdog's active-overlay tag are not — they are cleared by
            // this call, which every other exit from this function already makes.
            // Skipping it here left the app wearing a login overlay with nothing
            // behind it.
            crate::commands::twitch::dismiss_login_overlay(&app, LABEL);
            return Err(anyhow!("Sign-in was cancelled"));
        }
        if let Some(channels) = follows_via_cookies(&app, LABEL).await {
            report = Some(KickImportReport {
                status: "ok".to_string(),
                channels,
                shape: None,
            });
            break;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if report.is_none() {
        log::warn!("[Kick] no kick.com session appeared; continuing to authorization");
    }

    // Leg 2: consent, in the SAME overlay. Already signed in, so this is usually
    // one click, and the redirect lands on the loopback exactly as before.
    if let Ok(url) = auth_url.parse() {
        let _ = win.navigate(url);
    }
    match crate::services::kick_auth_service::finish_auth(auth_pending).await {
        Ok(()) => log::info!("[Kick] OAuth complete (in-app)"),
        Err(e) => log::warn!("[Kick] OAuth leg failed: {}", e),
    }

    // Closing goes through the overlay's own dismissal so React tears down its
    // chrome too; destroying the window alone would leave the frame on screen.
    crate::commands::twitch::dismiss_login_overlay(&app, LABEL);

    match report {
        Some(report) => {
            log::info!(
                "[Kick] sign-in {}: {} channel(s)",
                report.status,
                report.channels.len()
            );
            Ok(report)
        }
        // Authorization may still have succeeded, so this is a failed follow
        // import rather than necessarily a failed sign-in.
        None => Err(anyhow!("Signed in, but couldn't read your Kick follows")),
    }
}

#[cfg(not(windows))]
pub async fn sign_in() -> Result<KickImportReport> {
    Err(anyhow!("Kick sign-in is only wired for Windows so far"))
}

/// Read the kick.com session from the sign-in window and pull the follow list
/// from Rust.
///
/// This exists because page script CANNOT see an HttpOnly cookie. The injected
/// script built its `Authorization: Bearer` from `document.cookie`, so if Kick
/// marks `session_token` HttpOnly the header was simply absent, every request
/// came back 401, and the window sat there waiting for a session the user had
/// already established. The webview's cookie manager has no such blind spot, and
/// our rustls client already talks to kick.com successfully.
#[cfg(windows)]
async fn follows_via_cookies(app: &tauri::AppHandle, label: &str) -> Option<Vec<KickFollowedChannel>> {
    let jar = crate::services::youtube_auth_service::fetch_cookies_for_origin(
        app,
        label,
        // Empty list = every cookie on the origin, which is what a browser sends.
        &[],
        "https://kick.com",
    )
    .await
    .ok()?;
    if jar.is_empty() {
        return None;
    }
    // The bearer must be PERCENT-DECODED. Laravel Sanctum tokens are
    // `<id>|<secret>`, and the `|` arrives from the cookie store as `%7C`;
    // sending the raw value is a silent 401 with no hint as to why. The Cookie
    // header keeps the encoded form, because that is what a browser sends.
    let raw_session = jar.get("session_token")?;
    let session = urlencoding::decode(raw_session)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| raw_session.clone());
    let cookie_header = jar
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("; ");
    log::debug!(
        "[Kick] session cookie present ({} cookies, bearer {}decoded)",
        jar.len(),
        if session == *raw_session { "un" } else { "" }
    );

    let mut out: Vec<KickFollowedChannel> = Vec::new();
    let mut cursor = 0u32;
    for _ in 0..20 {
        let url = format!(
            "https://kick.com/api/v2/channels/followed?cursor={}",
            cursor
        );
        let mut headers = vec![
            ("Authorization", format!("Bearer {}", session)),
            ("Cookie", cookie_header.clone()),
        ];
        // Kick's own web client sends the CSRF token on these calls. It is not
        // required for a GET, but matching what the site does costs nothing and
        // removes a variable if the endpoint ever starts checking.
        if let Some(x) = jar.get("XSRF-TOKEN") {
            let decoded = urlencoding::decode(x)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| x.clone());
            headers.push(("X-XSRF-TOKEN", decoded));
        }
        let resp =
            crate::services::providers::kick::browser_get_with(&url, "following", &headers).await?;
        let body: serde_json::Value = resp.json().await.ok()?;
        let rows = body
            .get("channels")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        if rows.is_empty() {
            break;
        }
        for r in &rows {
            let Some(slug) = r
                .get("channel_slug")
                .and_then(|v| v.as_str())
                .or_else(|| r.get("slug").and_then(|v| v.as_str()))
            else {
                continue;
            };
            out.push(KickFollowedChannel {
                slug: slug.to_lowercase(),
                username: r
                    .get("user_username")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                profile_pic: r
                    .get("profile_picture")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                is_live: r.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false),
                subscribed: false,
            });
        }
        match body.get("nextCursor").and_then(|v| v.as_u64()) {
            Some(next) => cursor = next as u32,
            None => break,
        }
    }
    log::info!("[Kick] read {} follow(s) from the webview session", out.len());
    Some(out)
}
