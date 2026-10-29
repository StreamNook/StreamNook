//! Kick account sync: import the user's real follows and subscriptions.
//!
//! Kick's OFFICIAL API (api.kick.com) has no "channels this user follows"
//! endpoint — verified against their docs. Their website's own API does:
//! `GET kick.com/api/v2/channels/followed-page` (the /following page's list),
//! `/api/v2/channels/followed` (the sidebar's) and `/api/v2/user/subscriptions`,
//! all of which answer 401 rather than 404 when signed out, so they exist and
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

// The phone's in-app web view: Android's login overlay, the iPhone's sheet.
// Both expose the same helpers, so the sign-in and import below run one body.
#[cfg(target_os = "android")]
use crate::twitch_login_plugin as overlay;
#[cfg(target_os = "ios")]
use crate::services::ios_sheet as overlay;

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
    // The phone's overlay (Android) or sheet (iPhone) shares the app's cookie
    // jar, so a silent import is just a read of that jar; there is no window
    // to run.
    #[cfg(mobile)]
    {
        let _ = interactive;
        let app = app_handle().ok_or_else(|| anyhow!("app handle not available for Kick sync"))?;
        let jar = without_edge_cookies(jar_from_header(&overlay::cookies_for(&app, "https://kick.com")));
        return match follows_from_jar(&jar).await {
            Some(channels) => Ok(KickImportReport {
                status: "ok".to_string(),
                channels,
                shape: None,
            }),
            None => Err(anyhow!("not signed in to kick.com")),
        };
    }
    #[cfg(desktop)]
    {
        use crate::rt::WebviewWindowBuilder;
        use tauri::{Manager, WebviewUrl};

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
    let page_cap = FOLLOW_PAGE_CAP;
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
  // the places Kick's API has historically put it. null means the reply holds
  // no list at all: a changed reply, never "no follows".
  function rows(body) {{
    if (Array.isArray(body)) return body;
    if (!body) return null;
    if (Array.isArray(body.data)) return body.data;
    if (Array.isArray(body.channels)) return body.channels;
    if (body.data && Array.isArray(body.data.channels)) return body.data.channels;
    return null;
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
      || (e.channel && e.channel.username)
      || (e.channel && e.channel.user && e.channel.user.username)
      || e.name)) || null;
  }}
  function picOf(e) {{
    return (e && ((e.user && e.user.profile_pic)
      || e.profile_pic
      || (e.channel && e.channel.profile_image)
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

  // A 429 or a server error is asked again after a pause; anything else is
  // the answer.
  function getJsonPatiently(url, retries) {{
    return getJson(url).then(function(res) {{
      if ((res.status === 429 || res.status >= 500) && retries < 2) {{
        return new Promise(function(done) {{ setTimeout(done, retries === 0 ? 1500 : 4000); }})
          .then(function() {{ return getJsonPatiently(url, retries + 1); }});
      }}
      return res;
    }});
  }}

  // Kick pages with an offset cursor, the way kick.com's own pager reads it:
  // no cursor on the first page (pass null), then `nextCursor` sent back as
  // received while it is truthy. The sidebar list holds only 5 rows a page,
  // so the old 20-page stop capped imports at 100; PAGE_CAP only guards
  // against a cursor that never ends. A page that cannot be read marks the
  // whole result `failed`: a partial list must never be reported as the list,
  // because the sync replaces every imported follow with it.
  var PAGE_CAP = {page_cap};
  function collect(base, cursor, acc, shape, page, sent) {{
    var query = (cursor === null ? '' : 'cursor=' + encodeURIComponent(String(cursor)) + '&') + '_cb=' + Date.now();
    var done = function(extra) {{
      var out = {{ unauth: false, failed: false, items: acc, shape: shape, pages: page }};
      for (var k in extra) out[k] = extra[k];
      return out;
    }};
    return getJsonPatiently(base + (base.indexOf('?') >= 0 ? '&' : '?') + query, 0).then(function(res) {{
      if (res.status === 401 || res.status === 403) return done({{ unauth: true }});
      if (res.status !== 200) {{
        diag(base + ' page ' + (page + 1) + ' returned ' + res.status);
        return done({{ failed: true }});
      }}
      var list = rows(res.body);
      if (!list) {{
        // 200 but no list: the payload shape moved. Log its top-level keys so
        // the mismatch is visible instead of looking like "no follows".
        diag('no rows parsed from ' + base + '; keys: ' + (res.body && typeof res.body === 'object' ? Object.keys(res.body).join(',') : typeof res.body));
        return done({{ failed: true }});
      }}
      if (!shape && res.body && typeof res.body === 'object' && !Array.isArray(res.body)) {{
        shape = Object.keys(res.body).join(',');
      }}
      acc = acc.concat(list);
      page += 1;
      var next = Array.isArray(res.body) ? null : res.body.nextCursor;
      if (!next || next === '0' || list.length === 0) return done({{}});
      var key = String(next);
      if (sent[key]) {{
        diag(base + ' repeated a page cursor after ' + page + ' page(s)');
        return done({{}});
      }}
      sent[key] = true;
      if (page >= PAGE_CAP) {{
        diag(base + ' still had more after ' + page + ' pages');
        return done({{}});
      }}
      return collect(base, next, acc, shape, page, sent);
    }}).catch(function(e) {{
      diag(base + ' page ' + (page + 1) + ' failed: ' + e);
      return done({{ failed: true }});
    }});
  }}

  // kick.com's own Subscriptions page reads `new_subscriptions` from the v2
  // reply {{ subscriptions, new_subscriptions, new_gifts }}; `subscriptions` is
  // read too in case older ones are listed there, and `new_gifts` are subs
  // given to other people. One unpaged reply. v1 (a bare array) is only the
  // fallback. Resolves to the current rows, [] when neither can be read.
  function readSubs() {{
    return getJsonPatiently('https://kick.com/api/v2/user/subscriptions', 0).then(function(res) {{
      var list = res.status === 200 ? subRows(res.body) : null;
      if (list) return list;
      diag('subscriptions v2 gave ' + res.status + (res.status === 200 ? ' with no list' : '') + '; trying v1');
      return getJsonPatiently('https://kick.com/api/v1/subscriptions', 0).then(function(v1) {{
        return (v1.status === 200 && subRows(v1.body)) || [];
      }});
    }}).catch(function(e) {{ diag('subscriptions read failed: ' + e); return []; }});
  }}
  function subRows(body) {{
    if (Array.isArray(body)) return body.filter(currentSub);
    if (!body || typeof body !== 'object') return null;
    var fresh = Array.isArray(body.new_subscriptions) ? body.new_subscriptions : null;
    var older = Array.isArray(body.subscriptions) ? body.subscriptions : null;
    if (!fresh && !older) return null;
    return (fresh || []).concat(older || []).filter(currentSub);
  }}
  // A canceled subscription keeps its benefits until it runs out; an expired
  // one is over.
  function currentSub(s) {{
    if (!s || s.status === 'expired') return false;
    if (!s.expires_at) return true;
    var ends = Date.parse(s.expires_at);
    return isNaN(ends) || ends > Date.now();
  }}

  // The /following page's list holds far more rows a page than the sidebar's,
  // so it goes first; the sidebar list is read only when it gives no page.
  function readFollows() {{
    return collect('https://kick.com/api/v2/channels/followed-page', null, [], null, 0, {{}}).then(function(r) {{
      if (r.pages > 0) return r;
      diag('followed-page gave no page (' + (r.unauth ? 'not signed in' : 'failed') + '); reading the sidebar list');
      return collect('https://kick.com/api/v2/channels/followed', null, [], null, 0, {{}});
    }});
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
    readFollows().then(function(followed) {{
      if (followed.unauth && followed.pages === 0) {{
        // Not signed in YET — the user may still be mid-login. Fall back into
        // the waiting loop rather than ending the flow; it only gives up once
        // the whole window expires.
        diag('attempt ' + tries + ': not authenticated yet');
        if (waited >= WAIT_SECONDS) {{ report('unauthenticated', [], null); return; }}
        waited += 3;
        setTimeout(waitForSession, 3000);
        return;
      }}
      if (followed.failed || followed.unauth) {{
        diag('follow list read stopped after ' + followed.pages + ' page(s); reporting an error, not a partial list');
        report('error', [], followed.shape);
        return;
      }}
      diag('authenticated; followed rows: ' + followed.items.length + ' over ' + followed.pages + ' page(s)');
      // Subscriptions are a bonus: never let their failure lose the follow list.
      readSubs().then(function(subItems) {{
        var subSlugs = {{}};
        subItems.forEach(function(s) {{ var sl = slugOf(s); if (sl) subSlugs[String(sl).toLowerCase()] = true; }});
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
        subItems.forEach(function(s) {{
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
#[cfg(desktop)]
pub async fn sign_in() -> Result<KickImportReport> {
    use crate::services::sign_in_profile;
    use tauri::Manager;

    let app = app_handle().ok_or_else(|| anyhow!("app handle not available for Kick sign-in"))?;

    // With no Kick connection this is a fresh sign-in, and the profile may still
    // hold the kick.com and id.kick.com sessions of an account that signed out.
    // Taken, they would hand that account straight back: its follow list on
    // leg 1's first poll and its consent one click later. So the overlay opens
    // blank and shows the login page only once they are gone (see
    // `sign_in_profile`).
    let fresh = !crate::services::kick_auth_service::is_connected();

    // Hand the overlay the login page. React measures the app body and mounts the
    // webview at that rect; `kick-account` selects Kick's own cookie jar.
    crate::commands::twitch::emit_overlay_open_with(
        &app,
        LOGIN_LABEL,
        sign_in_profile::opening_url(fresh, LOGIN_URL),
        "fullbody",
        Some("kick-account"),
    )
    .map_err(|e| {
        log::warn!("[Kick] could not ask for the sign-in window: {}", e);
        anyhow!("The Kick sign-in window could not open.")
    })?;

    // The overlay mounts asynchronously (Rust asks, React measures, Rust builds),
    // so wait for the window to exist before addressing it.
    let mut win = None;
    for _ in 0..60 {
        if let Some(w) = app.get_webview_window(LOGIN_LABEL) {
            win = Some(w);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let win = win.ok_or_else(|| {
        crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_LABEL);
        log::warn!("[Kick] the sign-in overlay never mounted");
        anyhow!("The Kick sign-in window could not open.")
    })?;

    // Belt and braces: the session the profile held when this began is never
    // mistaken for this sign-in, whatever the clear managed.
    let stale = if fresh {
        sign_in_profile::begin_fresh(&app, &win, &profile(&app), LOGIN_URL).await
    } else {
        None
    };

    // Leg 1: wait for the site session by polling the webview's cookie jar. That
    // jar sees HttpOnly cookies, which page script cannot, so it is the only
    // reliable signal that the user is actually signed in.
    let mut report: Option<KickImportReport> = None;
    for _ in 0..150 {
        if app.get_webview_window(LOGIN_LABEL).is_none() {
            log::info!("[Kick] sign-in overlay dismissed by the user");
            // The child webview is gone, but the React overlay frame and the
            // ui_hang_watchdog's active-overlay tag are not — they are cleared by
            // this call, which every other exit from this function already makes.
            // Skipping it here left the app wearing a login overlay with nothing
            // behind it.
            crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_LABEL);
            return Err(anyhow!("Sign-in was cancelled"));
        }
        if let Some(channels) = follows_via_cookies(&app, LOGIN_LABEL, stale.as_deref()).await {
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
        // Consent authorizes whoever id.kick.com has signed in, and cannot be
        // held to a new session the way leg 1 is. With the signed-out account's
        // session still in the profile and no new one seen, that is the account
        // the user just left, so the sign-in stops here.
        if let Some(old) = stale.as_deref() {
            if still_holds(&app, old).await {
                crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_LABEL);
                return Err(anyhow!(
                    "The Kick sign-in window kept the account you signed out of. Try connecting again."
                ));
            }
        }
        log::warn!("[Kick] no kick.com session appeared; continuing to authorization");
    }

    // Leg 2: consent, in the SAME overlay. Already signed in, so this is usually
    // one click, and the window's navigation handler takes the redirect.
    //
    // Prepared HERE rather than before the window, which is the ordering that
    // shipped the bug: `begin_auth` could fail, and failing before the emit meant
    // the sign-in ended with no window ever appearing and nothing on screen to
    // say why. Nothing above this line can fail for a reason the user could act
    // on, so the window is now unconditional and only the consent leg is at risk.
    match crate::services::kick_auth_service::begin_auth().await {
        Ok((auth_url, auth_pending)) => {
            if let Ok(url) = auth_url.parse() {
                let _ = win.navigate(url);
            }
            match crate::services::kick_auth_service::finish_auth(auth_pending).await {
                Ok(()) => log::info!("[Kick] OAuth complete (in-app)"),
                Err(e) => log::warn!("[Kick] OAuth leg failed: {}", e),
            }
        }
        Err(e) => log::warn!("[Kick] could not start the authorization leg: {}", e),
    }

    // Closing goes through the overlay's own dismissal so React tears down its
    // chrome too; destroying the window alone would leave the frame on screen.
    crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_LABEL);

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

/// The phone's sign-in: the same two legs as desktop, in the native login
/// overlay (Android) or sheet (iPhone). Site login first (consent at
/// id.kick.com does not leave kick.com cookies), then consent in the same
/// overlay, whose redirect the overlay catches and this function relays to
/// `kick_auth_service`.
#[cfg(mobile)]
pub async fn sign_in() -> Result<KickImportReport> {
    const TITLE: &str = "Sign in to Kick";
    let app = app_handle().ok_or_else(|| anyhow!("app handle not available for Kick sign-in"))?;
    overlay::open_overlay(&app, "https://kick.com/login", TITLE).map_err(|e| {
        log::warn!("[Kick] could not open the sign-in overlay: {}", e);
        anyhow!("The Kick sign-in window could not open.")
    })?;

    // Leg 1: the site session. The overlay's jar sees HttpOnly cookies.
    let mut report: Option<KickImportReport> = None;
    // Signed in but the list will not load (a 401/403 from the site API) is a
    // different state from "not signed in yet": waiting cannot fix it, and
    // polling on would leave the person staring at kick.com for five minutes.
    let mut read_failures = 0u8;
    for _ in 0..150 {
        if !overlay::overlay_is_open(&app) {
            log::info!("[Kick] sign-in overlay dismissed by the user");
            return Err(anyhow!("Sign-in was cancelled"));
        }
        let jar = without_edge_cookies(jar_from_header(&overlay::cookies_for(&app, "https://kick.com")));
        let has_session = jar.contains_key("session_token");
        if let Some(channels) = follows_from_jar(&jar).await {
            report = Some(KickImportReport {
                status: "ok".to_string(),
                channels,
                shape: None,
            });
            break;
        }
        if has_session {
            read_failures += 1;
            if read_failures >= 3 {
                log::warn!("[Kick] signed in to kick.com but the follow list would not load; continuing to authorization");
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    if report.is_none() {
        log::warn!("[Kick] no kick.com follow list; continuing to authorization");
    }

    // Leg 2: consent in the same overlay. The overlay takes the localhost
    // redirect off the navigation; this relays it to the waiting exchange.
    match crate::services::kick_auth_service::begin_auth().await {
        Ok((auth_url, auth_pending)) => {
            let _ = overlay::open_overlay(&app, &auth_url, TITLE);
            let relay = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    // Open flag FIRST, then the redirect. The overlay stores the
                    // redirect before it closes itself, so once a closed overlay
                    // is seen here, the take below is guaranteed to see any
                    // redirect that closed it. The reverse order loses a sign-in
                    // whenever the capture lands between the two reads.
                    let open = overlay::overlay_is_open(&app);
                    let url = overlay::take_kick_redirect(&app);
                    if !url.is_empty() {
                        if !crate::services::kick_auth_service::capture_redirect(&url) {
                            log::warn!("[Kick] consent redirect arrived with nothing waiting");
                        }
                        // finish_auth owns the outcome from here.
                        std::future::pending::<()>().await;
                    }
                    if !open {
                        return;
                    }
                }
            };
            tokio::select! {
                r = crate::services::kick_auth_service::finish_auth(auth_pending) => match r {
                    Ok(()) => log::info!("[Kick] OAuth complete (phone overlay)"),
                    Err(e) => log::warn!("[Kick] OAuth leg failed: {}", e),
                },
                _ = relay => log::info!("[Kick] consent overlay closed before authorizing"),
            }
        }
        Err(e) => log::warn!("[Kick] could not start the authorization leg: {}", e),
    }
    overlay::close_overlay(&app);

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


/// The follow list, read with the site session in the sign-in overlay's jar.
/// Never with `stale`, the session the profile kept from before the sign-in
/// began.
#[cfg(desktop)]
async fn follows_via_cookies(
    app: &crate::rt::AppHandle,
    label: &str,
    stale: Option<&str>,
) -> Option<Vec<KickFollowedChannel>> {
    let jar = crate::services::youtube_auth_service::fetch_cookies_for_origin(
        app,
        label,
        // Empty list = every cookie on the origin, which is what a browser sends.
        &[],
        "https://kick.com",
    )
    .await
    .ok()?;
    if !is_new_site_session(&jar, stale) {
        return None;
    }
    follows_from_jar(&jar).await
}

/// Whether the sign-in overlay's jar still holds the kick.com session `old`.
#[cfg(desktop)]
async fn still_holds(app: &crate::rt::AppHandle, old: &str) -> bool {
    crate::services::youtube_auth_service::fetch_cookies_for_origin(
        app,
        LOGIN_LABEL,
        &[],
        "https://kick.com",
    )
    .await
    .is_ok_and(|jar| site_session(&jar) == Some(old))
}

/// The kick.com site session's cookie, None when signed out.
fn site_session(jar: &HashMap<String, String>) -> Option<&str> {
    jar.get("session_token").map(String::as_str).filter(|v| !v.is_empty())
}

/// A site session a sign-in may take: one exists, and it is not the one the
/// profile kept from before the sign-in began.
fn is_new_site_session(jar: &HashMap<String, String>, stale: Option<&str>) -> bool {
    crate::services::sign_in_profile::is_new_session(site_session(jar), stale)
}

/// The sign-in overlay's window label, which `mount_twitch_overlay` also knows
/// it by: it takes the consent redirect off that window's navigation.
#[cfg(desktop)]
const LOGIN_LABEL: &str = "kick-login";
#[cfg(desktop)]
const LOGIN_URL: &str = "https://kick.com/login";

/// The Kick sign-in profile, for signing it out and for starting a fresh
/// sign-in. Clearing it clears both halves at once: kick.com's session and
/// id.kick.com's, the one consent runs on.
#[cfg(desktop)]
fn profile(app: &crate::rt::AppHandle) -> crate::services::sign_in_profile::SignInProfile {
    crate::services::sign_in_profile::SignInProfile {
        dir: crate::services::providers::kick::account_profile_dir(app),
        overlay_label: LOGIN_LABEL,
        sign_out_label: "kick-sign-out",
        origin: "https://kick.com",
        // id.kick.com, where consent runs, included.
        sites: &["kick.com"],
        session: site_session,
        tag: "Kick",
    }
}

/// Sign the Kick sign-in profile out, through a live webview on it: the site
/// half of a Kick sign-out, which `kick_auth_service::disconnect` (the OAuth
/// token) does not cover.
#[cfg(desktop)]
pub async fn clear_site_session(app: &crate::rt::AppHandle) {
    crate::services::sign_in_profile::sign_out(&profile(app)).await;
}

/// Parse a `Cookie`-style header (`a=b; c=d`) into a jar. Values stay encoded,
/// exactly as a browser would send them; `follows_from_jar` decodes the bearer.
fn jar_from_header(raw: &str) -> HashMap<String, String> {
    raw.split(';')
        .filter_map(|part| {
            let (k, v) = part.split_once('=')?;
            let k = k.trim();
            (!k.is_empty()).then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Cloudflare's clearance cookies are bound to the browser that earned them
/// (its user agent and TLS handshake). The phone's overlay is a mobile WebView
/// while our requests present as desktop Chrome over rustls, so replayed they
/// can only mismatch. Leaving them out makes the request match the channel
/// lookup, which already succeeds with no cookies at all.
fn without_edge_cookies(mut jar: HashMap<String, String>) -> HashMap<String, String> {
    jar.retain(|k, _| !matches!(k.as_str(), "cf_clearance" | "__cf_bm" | "_cfuvid"));
    jar
}

/// Pull the follow list with a kick.com site session read from a cookie jar.
/// None means the jar holds no usable session yet, or the list could not be
/// read whole this time; the sign-in loops poll on it either way.
///
/// This exists because page script CANNOT see an HttpOnly cookie. The injected
/// script built its `Authorization: Bearer` from `document.cookie`, so if Kick
/// marks `session_token` HttpOnly the header was simply absent, every request
/// came back 401, and the window sat there waiting for a session the user had
/// already established. The webview's cookie manager has no such blind spot, and
/// our rustls client already talks to kick.com successfully.
async fn follows_from_jar(jar: &HashMap<String, String>) -> Option<Vec<KickFollowedChannel>> {
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

    let mut headers = vec![
        ("Authorization", format!("Bearer {}", session)),
        ("Cookie", cookie_header),
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
    let headers = &headers;
    read_account(
        move |list, cursor| {
            let url = follow_page_url(list, cursor.as_deref());
            async move { fetch_follow_page(&url, headers).await }
        },
        &FOLLOW_RETRY_PAUSES,
        chrono::Utc::now(),
    )
    .await
}

/// The follow list with the user's subscriptions folded in. Subscriptions are
/// a bonus: a failed read of them never costs the follow list.
async fn read_account<F, Fut>(
    mut fetch: F,
    pauses: &[Duration],
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Vec<KickFollowedChannel>>
where
    F: FnMut(&'static str, Option<String>) -> Fut,
    Fut: std::future::Future<Output = FollowFetch>,
{
    let mut channels = imported_follows(read_follows(&mut fetch, pauses).await)?;
    let subs = read_subscriptions(&mut fetch, pauses, now).await;
    fold_subscriptions(&mut channels, subs);
    Some(channels)
}

// --- Reading the whole follow list ------------------------------------------
//
// Kick pages follows with an OFFSET cursor: the first page goes out with no
// cursor, each reply carries `nextCursor` (5, 10, ...), and the last page has
// none. The sidebar list (`/followed`) holds a fixed 5 rows a page and ignores
// any size parameter, so the old 20-page loop could never read past 100. The
// /following page's list (`/followed-page`) holds far more a page, so it goes
// first and the sidebar list is only the fallback. Verified live 2026-10-29.

/// The follow lists, in the order they are tried.
const FOLLOW_LISTS: [&str; 2] = ["/api/v2/channels/followed-page", "/api/v2/channels/followed"];

/// A guard against a cursor that never ends, far above any real list: even
/// the 5-row sidebar list reaches 2,500 follows before it.
const FOLLOW_PAGE_CAP: usize = 500;

/// Waits before re-asking for a page Kick refused with "slow down" (429) or a
/// server error. One entry per retry.
const FOLLOW_RETRY_PAUSES: [Duration; 2] = [Duration::from_millis(1500), Duration::from_secs(4)];

/// One page request's outcome, as the pager sees it.
#[derive(Debug, Clone)]
enum FollowFetch {
    /// A 200 with a JSON body.
    Page(serde_json::Value),
    /// 401 or 403: no usable session.
    Unauthorized,
    /// Worth asking again shortly: 429, a 5xx, no answer, or a cut-off body.
    Busy(String),
    /// Asking again will not help (404 and the like).
    Failed(String),
}

/// How reading one follow list ended.
#[derive(Debug, Clone, PartialEq)]
enum WalkEnd {
    /// Kick said there are no more pages.
    Complete,
    /// Kick sent back a cursor it had already given. Everything before it was
    /// read; asking again would only loop.
    RepeatedCursor,
    /// `FOLLOW_PAGE_CAP` pages read and Kick still offered more.
    Capped,
    /// No usable session.
    Unauthorized,
    /// A page could not be read, so the list is not known to be whole.
    Failed(String),
}

#[derive(Debug)]
struct FollowWalk {
    list: &'static str,
    channels: Vec<KickFollowedChannel>,
    pages: usize,
    end: WalkEnd,
}

fn follow_page_url(list: &str, cursor: Option<&str>) -> String {
    match cursor {
        Some(c) => format!("https://kick.com{}?cursor={}", list, urlencoding::encode(c)),
        None => format!("https://kick.com{}", list),
    }
}

async fn fetch_follow_page(url: &str, headers: &[(&str, String)]) -> FollowFetch {
    use crate::services::providers::kick::{browser_get_with, GetFailure};
    match browser_get_with(url, "following", headers).await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(body) => FollowFetch::Page(body),
            Err(e) => FollowFetch::Busy(format!("unreadable reply: {}", e)),
        },
        Err(GetFailure::Status(401 | 403)) => FollowFetch::Unauthorized,
        Err(GetFailure::Status(s)) if s == 429 || s >= 500 => FollowFetch::Busy(format!("HTTP {}", s)),
        Err(GetFailure::Status(s)) => FollowFetch::Failed(format!("HTTP {}", s)),
        Err(GetFailure::Network) => FollowFetch::Busy("no answer".to_string()),
    }
}

/// The cursor for the next page, as kick.com's own pager reads it: sent back
/// as received while it is truthy. A number, or a string in case Kick ever
/// switches to opaque cursors. `0`, `""`, `"0"`, null and absent all end the list.
fn next_cursor(body: &serde_json::Value) -> Option<String> {
    use serde_json::Value;
    match body.get("nextCursor")? {
        Value::Number(n) => n.as_f64().is_some_and(|f| f > 0.0).then(|| n.to_string()),
        Value::String(s) => {
            let s = s.trim();
            (!s.is_empty() && s != "0").then(|| s.to_string())
        }
        _ => None,
    }
}

/// The page's rows, wherever Kick put them. None when the reply holds no list
/// at all, which is a changed reply and never "no follows".
fn follow_rows(body: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    body.as_array()
        .or_else(|| body.get("channels").and_then(|v| v.as_array()))
        .or_else(|| body.get("data").and_then(|v| v.as_array()))
        .or_else(|| body.pointer("/data/channels").and_then(|v| v.as_array()))
}

fn followed_channel(row: &serde_json::Value) -> Option<KickFollowedChannel> {
    let text = |path: &str| {
        row.pointer(path)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let slug = text("/channel_slug")
        .or_else(|| text("/slug"))
        .or_else(|| text("/channel/slug"))?;
    Some(KickFollowedChannel {
        slug: slug.to_lowercase(),
        username: text("/user_username").or_else(|| text("/username")).map(str::to_string),
        profile_pic: text("/profile_picture").or_else(|| text("/profile_pic")).map(str::to_string),
        is_live: row.get("is_live").and_then(|v| v.as_bool()).unwrap_or(false),
        subscribed: false,
    })
}

/// The top-level keys of a reply, for the log when it has no list.
fn reply_keys(body: &serde_json::Value) -> String {
    match body.as_object() {
        Some(o) => o.keys().cloned().collect::<Vec<_>>().join(","),
        None => "not an object".to_string(),
    }
}

/// One page, asking again after each pause while Kick says "slow down".
async fn fetch_patiently<F, Fut>(
    fetch: &mut F,
    list: &'static str,
    cursor: Option<String>,
    pauses: &[Duration],
) -> Result<serde_json::Value, WalkEnd>
where
    F: FnMut(&'static str, Option<String>) -> Fut,
    Fut: std::future::Future<Output = FollowFetch>,
{
    let mut retries = 0;
    loop {
        match fetch(list, cursor.clone()).await {
            FollowFetch::Page(body) => return Ok(body),
            FollowFetch::Unauthorized => return Err(WalkEnd::Unauthorized),
            FollowFetch::Failed(why) => return Err(WalkEnd::Failed(why)),
            FollowFetch::Busy(why) => match pauses.get(retries) {
                Some(pause) => {
                    retries += 1;
                    log::debug!("[Kick] {} busy ({}); asking again in {:?}", list, why, pause);
                    tokio::time::sleep(*pause).await;
                }
                None => return Err(WalkEnd::Failed(why)),
            },
        }
    }
}

/// Every page of one follow list, each channel once.
async fn walk_follow_list<F, Fut>(fetch: &mut F, list: &'static str, pauses: &[Duration]) -> FollowWalk
where
    F: FnMut(&'static str, Option<String>) -> Fut,
    Fut: std::future::Future<Output = FollowFetch>,
{
    let mut channels: Vec<KickFollowedChannel> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut sent = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    let end = loop {
        if pages >= FOLLOW_PAGE_CAP {
            break WalkEnd::Capped;
        }
        let body = match fetch_patiently(fetch, list, cursor.clone(), pauses).await {
            Ok(body) => body,
            Err(end) => break end,
        };
        let Some(rows) = follow_rows(&body) else {
            break WalkEnd::Failed(format!("reply holds no follow list (keys: {})", reply_keys(&body)));
        };
        pages += 1;
        let before = channels.len();
        for row in rows {
            if let Some(ch) = followed_channel(row) {
                if seen.insert(ch.slug.clone()) {
                    channels.push(ch);
                }
            }
        }
        let next = next_cursor(&body);
        log::debug!(
            "[Kick] {} page {}: {} row(s), {} new, {}",
            list,
            pages,
            rows.len(),
            channels.len() - before,
            if next.is_some() { "more to come" } else { "last page" }
        );
        // A cursor on an empty page leads nowhere.
        let Some(next) = next.filter(|_| !rows.is_empty()) else {
            break WalkEnd::Complete;
        };
        if !sent.insert(next.clone()) {
            break WalkEnd::RepeatedCursor;
        }
        cursor = Some(next);
    };
    FollowWalk { list, channels, pages, end }
}

/// The whole follow list: the /following page's list, or the sidebar list
/// when that one gives no page at all (Kick moved it, or its edge refused it).
/// A list that fails partway is not swapped for the other: that would restart
/// the read from scratch on the list with the smaller pages.
async fn read_follows<F, Fut>(mut fetch: F, pauses: &[Duration]) -> FollowWalk
where
    F: FnMut(&'static str, Option<String>) -> Fut,
    Fut: std::future::Future<Output = FollowFetch>,
{
    let mut walk = walk_follow_list(&mut fetch, FOLLOW_LISTS[0], pauses).await;
    for &list in &FOLLOW_LISTS[1..] {
        let gave_nothing =
            walk.pages == 0 && matches!(walk.end, WalkEnd::Unauthorized | WalkEnd::Failed(_));
        if !gave_nothing {
            break;
        }
        log::debug!("[Kick] {} gave no page ({:?}); reading {}", walk.list, walk.end, list);
        walk = walk_follow_list(&mut fetch, list, pauses).await;
    }
    walk
}

/// The import's answer from a finished read. A read that failed partway is
/// never handed back as the follow list: the sync REPLACES every imported Kick
/// follow with it, so a partial list would quietly unfollow the rest. None
/// tells the sign-in loops to try again. The two guard stops (a repeated
/// cursor, the page cap) keep what they read, because reading again would
/// only stop at the same place.
fn imported_follows(walk: FollowWalk) -> Option<Vec<KickFollowedChannel>> {
    let FollowWalk { list, channels, pages, end } = walk;
    match end {
        WalkEnd::Complete => {
            log::info!(
                "[Kick] read {} follow(s) from the webview session ({} page(s) of {})",
                channels.len(),
                pages,
                list
            );
            Some(channels)
        }
        WalkEnd::RepeatedCursor => {
            log::warn!(
                "[Kick] {} repeated a page cursor after {} page(s); kept the {} follow(s) read before it",
                list,
                pages,
                channels.len()
            );
            Some(channels)
        }
        WalkEnd::Capped => {
            log::warn!(
                "[Kick] {} still had more after {} pages; kept the first {} follow(s)",
                list,
                pages,
                channels.len()
            );
            Some(channels)
        }
        WalkEnd::Unauthorized => {
            log::debug!("[Kick] {} answered not signed in after {} page(s)", list, pages);
            None
        }
        WalkEnd::Failed(why) => {
            log::warn!(
                "[Kick] follow list read stopped on {} page {} ({}); nothing imported this time",
                list,
                pages + 1,
                why
            );
            None
        }
    }
}

// --- Subscriptions ----------------------------------------------------------
//
// kick.com's own Subscriptions page (/transactions/subscriptions) reads
// `new_subscriptions` from `/api/v2/user/subscriptions`, one unpaged reply of
// `{ subscriptions, new_subscriptions, new_gifts }`. A row is
// `{ channel: { slug, username, profile_image }, status, expires_at,
// provider, provider_id, created_at }`, status one of active, canceled and
// expired. `subscriptions` is read as well in case older subscriptions are
// still listed there; `new_gifts` are subs given to other people. Read from
// the site's bundle 2026-10-29.

/// The subscription lists, in the order they are tried. v1 is a bare array
/// and only the fallback.
const SUBSCRIPTION_LISTS: [&str; 2] = ["/api/v2/user/subscriptions", "/api/v1/subscriptions"];

/// The channels the user is subscribed to right now. Never fails: a list that
/// cannot be read is logged and counts as none.
async fn read_subscriptions<F, Fut>(
    mut fetch: F,
    pauses: &[Duration],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<KickFollowedChannel>
where
    F: FnMut(&'static str, Option<String>) -> Fut,
    Fut: std::future::Future<Output = FollowFetch>,
{
    for list in SUBSCRIPTION_LISTS {
        match fetch_patiently(&mut fetch, list, None, pauses).await {
            Ok(body) => match subscribed_channels(&body, now) {
                Some(subs) => {
                    log::debug!("[Kick] {}: {} current subscription(s)", list, subs.len());
                    return subs;
                }
                None => log::debug!(
                    "[Kick] {} holds no subscription list (keys: {})",
                    list,
                    reply_keys(&body)
                ),
            },
            Err(end) => log::debug!("[Kick] {} could not be read ({:?})", list, end),
        }
    }
    Vec::new()
}

/// The current subscriptions in a reply, each channel once. None when the
/// reply holds no subscription list at all.
fn subscribed_channels(
    body: &serde_json::Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Vec<KickFollowedChannel>> {
    let rows: Vec<&serde_json::Value> = match body {
        serde_json::Value::Array(rows) => rows.iter().collect(),
        serde_json::Value::Object(o) => {
            let lists: Vec<&Vec<serde_json::Value>> = ["new_subscriptions", "subscriptions"]
                .iter()
                .filter_map(|k| o.get(*k).and_then(|v| v.as_array()))
                .collect();
            if lists.is_empty() {
                return None;
            }
            lists.into_iter().flatten().collect()
        }
        _ => return None,
    };
    let mut seen = std::collections::HashSet::new();
    Some(
        rows.into_iter()
            .filter(|row| subscription_is_current(row, now))
            .filter_map(subscribed_channel)
            .filter(|ch| seen.insert(ch.slug.clone()))
            .collect(),
    )
}

/// A canceled subscription keeps its benefits until it runs out; an expired
/// one is over.
fn subscription_is_current(row: &serde_json::Value, now: chrono::DateTime<chrono::Utc>) -> bool {
    if row.get("status").and_then(|v| v.as_str()) == Some("expired") {
        return false;
    }
    match row
        .get("expires_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
    {
        Some(ends) => ends.with_timezone(&chrono::Utc) > now,
        None => true,
    }
}

fn subscribed_channel(row: &serde_json::Value) -> Option<KickFollowedChannel> {
    let text = |path: &str| {
        row.pointer(path)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let username = text("/channel/username").or_else(|| text("/channel/user/username"));
    // kick.com links a subscription to `slug ?? username`, so do the same.
    let slug = text("/channel/slug")
        .or_else(|| text("/channel_slug"))
        .or(username)?;
    Some(KickFollowedChannel {
        slug: slug.to_lowercase(),
        username: username.or_else(|| text("/user_username")).map(str::to_string),
        profile_pic: text("/channel/profile_image")
            .or_else(|| text("/channel/profile_pic"))
            .or_else(|| text("/channel/user/profile_pic"))
            .map(str::to_string),
        is_live: false,
        subscribed: true,
    })
}

/// Mark followed channels the user subscribes to, and add the ones they
/// subscribe to without following: they clearly care about those too.
fn fold_subscriptions(channels: &mut Vec<KickFollowedChannel>, subs: Vec<KickFollowedChannel>) {
    for sub in subs {
        match channels.iter_mut().find(|ch| ch.slug == sub.slug) {
            Some(followed) => followed.subscribed = true,
            None => channels.push(sub),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jar_from_header_keeps_encoded_values_and_skips_junk() {
        let jar = jar_from_header("session_token=12%7Cabc; XSRF-TOKEN=x%3D; ; novalue; a = b ");
        assert_eq!(jar.get("session_token").map(String::as_str), Some("12%7Cabc"));
        assert_eq!(jar.get("XSRF-TOKEN").map(String::as_str), Some("x%3D"));
        assert_eq!(jar.get("a").map(String::as_str), Some("b"));
        assert_eq!(jar.len(), 3);
    }

    #[test]
    fn edge_cookies_are_never_replayed() {
        let jar = without_edge_cookies(jar_from_header(
            "session_token=t; cf_clearance=c; __cf_bm=b; _cfuvid=u; XSRF-TOKEN=x",
        ));
        let mut keys: Vec<_> = jar.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["XSRF-TOKEN", "session_token"]);
    }

    #[test]
    fn a_site_session_is_session_token_and_nothing_less() {
        let guest = jar_from_header("XSRF-TOKEN=x; kick_session=k; cf_clearance=c");
        assert_eq!(site_session(&guest), None, "a signed-out browser has cookies too");
        assert_eq!(site_session(&jar_from_header("session_token=")), None);
        let signed_in = jar_from_header("session_token=12%7Cabc; XSRF-TOKEN=x");
        assert_eq!(site_session(&signed_in), Some("12%7Cabc"));
    }

    #[test]
    fn a_signed_out_accounts_site_session_is_never_taken_for_a_new_sign_in() {
        let old = jar_from_header("session_token=12%7Cold");
        assert!(!is_new_site_session(&old, Some("12%7Cold")), "the leftover session");
        assert!(is_new_site_session(&old, None), "a clean profile takes any session");
        let new = jar_from_header("session_token=34%7Cnew");
        assert!(is_new_site_session(&new, Some("12%7Cold")), "signing in replaced it");
        let out = jar_from_header("XSRF-TOKEN=x");
        assert!(!is_new_site_session(&out, Some("12%7Cold")), "signed out is not new");
    }
}

#[cfg(test)]
mod follow_pager_tests {
    use super::*;
    use serde_json::json;

    const PAGE: &str = "/api/v2/channels/followed-page";
    const SIDEBAR: &str = "/api/v2/channels/followed";
    const NO_PAUSE: [Duration; 2] = [Duration::ZERO, Duration::ZERO];

    /// One page of a list of `total` follows, `size` rows a page, answered the
    /// way Kick answers: an offset cursor, and none on the last page.
    fn kick_page(total: usize, size: usize, cursor: Option<&str>) -> FollowFetch {
        let from = cursor.map(|c| c.parse::<usize>().expect("an offset cursor")).unwrap_or(0);
        let to = from.saturating_add(size).min(total);
        let channels: Vec<_> = (from..to)
            .map(|i| {
                json!({
                    "channel_slug": format!("Chan{}", i),
                    "user_username": format!("Chan{}", i),
                    "profile_picture": "https://files.kick.com/p.webp",
                    "is_live": i == 0,
                    "viewer_count": 0,
                })
            })
            .collect();
        let mut body = json!({ "channels": channels });
        if to < total {
            body["nextCursor"] = json!(to);
        }
        FollowFetch::Page(body)
    }

    fn slugs(walk: &FollowWalk) -> Vec<&str> {
        walk.channels.iter().map(|c| c.slug.as_str()).collect()
    }

    #[tokio::test]
    async fn the_five_row_sidebar_list_is_read_past_a_hundred() {
        let walk = read_follows(
            |list, cursor: Option<String>| {
                std::future::ready(match list {
                    PAGE => FollowFetch::Failed("HTTP 404".into()),
                    _ => kick_page(137, 5, cursor.as_deref()),
                })
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.list, SIDEBAR);
        assert_eq!(walk.end, WalkEnd::Complete);
        assert_eq!(walk.pages, 28);
        assert_eq!(walk.channels.len(), 137, "the old 20-page loop stopped at 100");
        assert_eq!(walk.channels[136].slug, "chan136");
    }

    #[tokio::test]
    async fn the_following_page_list_goes_first_and_its_cursor_goes_back_as_received() {
        let mut calls = Vec::new();
        let walk = read_follows(
            |list, cursor: Option<String>| {
                calls.push((list, cursor.clone()));
                std::future::ready(kick_page(130, 50, cursor.as_deref()))
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.list, PAGE);
        assert_eq!(walk.channels.len(), 130);
        assert_eq!(
            calls,
            vec![
                (PAGE, None),
                (PAGE, Some("50".to_string())),
                (PAGE, Some("100".to_string()))
            ],
            "the first page sends no cursor, like kick.com's own pager"
        );
    }

    #[test]
    fn cursors_are_read_the_way_kick_com_reads_them() {
        assert_eq!(next_cursor(&json!({ "nextCursor": 5 })).as_deref(), Some("5"));
        assert_eq!(next_cursor(&json!({ "nextCursor": "100" })).as_deref(), Some("100"));
        assert_eq!(next_cursor(&json!({ "nextCursor": "eyJpZCI6NX0=" })).as_deref(), Some("eyJpZCI6NX0="));
        for end in [
            json!({}),
            json!({ "nextCursor": null }),
            json!({ "nextCursor": 0 }),
            json!({ "nextCursor": "" }),
            json!({ "nextCursor": "0" }),
            json!({ "nextCursor": false }),
            json!({ "nextCursor": -5 }),
        ] {
            assert_eq!(next_cursor(&end), None, "{} ends the list", end);
        }
    }

    #[test]
    fn the_first_page_has_no_cursor_and_later_cursors_are_url_encoded() {
        assert_eq!(follow_page_url(PAGE, None), "https://kick.com/api/v2/channels/followed-page");
        assert_eq!(
            follow_page_url(SIDEBAR, Some("10")),
            "https://kick.com/api/v2/channels/followed?cursor=10"
        );
        assert_eq!(
            follow_page_url(PAGE, Some("a b/c=")),
            "https://kick.com/api/v2/channels/followed-page?cursor=a%20b%2Fc%3D"
        );
    }

    #[tokio::test]
    async fn an_opaque_string_cursor_keeps_paging() {
        let walk = read_follows(
            |_, cursor: Option<String>| {
                std::future::ready(FollowFetch::Page(match cursor.as_deref() {
                    None => json!({ "channels": [{ "channel_slug": "a" }], "nextCursor": "p2==" }),
                    Some("p2==") => json!({ "channels": [{ "channel_slug": "b" }], "nextCursor": "p3==" }),
                    Some("p3==") => json!({ "channels": [{ "channel_slug": "c" }] }),
                    other => panic!("unexpected cursor {:?}", other),
                }))
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.end, WalkEnd::Complete);
        assert_eq!(slugs(&walk), ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn a_cursor_kick_already_gave_stops_the_read_instead_of_looping() {
        let mut calls = 0;
        let walk = read_follows(
            |_, _| {
                calls += 1;
                std::future::ready(FollowFetch::Page(
                    json!({ "channels": [{ "channel_slug": "same" }], "nextCursor": 5 }),
                ))
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.end, WalkEnd::RepeatedCursor);
        assert_eq!(calls, 2);
        assert_eq!(slugs(&walk), ["same"]);
    }

    #[tokio::test]
    async fn a_channel_seen_on_two_pages_is_kept_once() {
        // A channel going live mid-read moves up Kick's live-first order, so
        // an offset page can repeat a row.
        let walk = read_follows(
            |_, cursor: Option<String>| {
                std::future::ready(FollowFetch::Page(match cursor {
                    None => json!({ "channels": [{ "channel_slug": "A" }, { "channel_slug": "b" }], "nextCursor": 2 }),
                    Some(_) => json!({ "channels": [
                        { "channel_slug": "b" }, { "channel_slug": "c" }, { "slug": " " }, { "user_username": "noslug" }
                    ] }),
                }))
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(slugs(&walk), ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn a_list_that_fails_partway_imports_nothing() {
        let mut lists = Vec::new();
        let walk = read_follows(
            |list, cursor: Option<String>| {
                lists.push(list);
                std::future::ready(match cursor.as_deref() {
                    Some("100") => FollowFetch::Failed("HTTP 404".into()),
                    c => kick_page(300, 50, c),
                })
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.end, WalkEnd::Failed("HTTP 404".into()));
        assert_eq!(walk.pages, 2);
        assert!(
            lists.iter().all(|l| *l == PAGE),
            "a list that failed partway is not restarted on the other one"
        );
        assert!(imported_follows(walk).is_none(), "a partial list would unfollow the rest");
    }

    #[tokio::test]
    async fn a_slow_down_answer_is_asked_again_then_given_up_on() {
        let mut tries = 0;
        let walk = read_follows(
            |_, cursor: Option<String>| {
                tries += 1;
                std::future::ready(if tries == 2 {
                    FollowFetch::Busy("HTTP 429".into())
                } else {
                    kick_page(60, 50, cursor.as_deref())
                })
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.end, WalkEnd::Complete);
        assert_eq!(walk.channels.len(), 60);
        assert_eq!(tries, 3);

        // Busy past every pause on the first page counts as giving nothing, so
        // the sidebar list is read instead.
        let mut tries = 0;
        let walk = read_follows(
            |list, cursor: Option<String>| {
                tries += 1;
                std::future::ready(if list == PAGE {
                    FollowFetch::Busy("HTTP 503".into())
                } else {
                    kick_page(3, 5, cursor.as_deref())
                })
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(tries, 1 + NO_PAUSE.len() + 1);
        assert_eq!((walk.list, walk.channels.len()), (SIDEBAR, 3));
    }

    #[tokio::test]
    async fn the_sidebar_list_is_read_only_when_the_following_page_list_gives_nothing() {
        for refusal in [
            FollowFetch::Unauthorized,
            FollowFetch::Failed("HTTP 404".into()),
            FollowFetch::Page(json!({ "message": "Not Found" })),
        ] {
            let walk = read_follows(
                |list, cursor: Option<String>| {
                    std::future::ready(if list == PAGE {
                        refusal.clone()
                    } else {
                        kick_page(12, 5, cursor.as_deref())
                    })
                },
                &NO_PAUSE,
            )
            .await;
            assert_eq!(
                (walk.list, walk.channels.len(), walk.end),
                (SIDEBAR, 12, WalkEnd::Complete),
                "after {:?}",
                refusal
            );
        }
    }

    #[tokio::test]
    async fn an_account_that_follows_nobody_imports_an_empty_list() {
        let walk = read_follows(
            |_, _| std::future::ready(FollowFetch::Page(json!({ "channels": [] }))),
            &NO_PAUSE,
        )
        .await;
        assert_eq!((walk.list, &walk.end), (PAGE, &WalkEnd::Complete));
        assert_eq!(imported_follows(walk).map(|c| c.len()), Some(0));
    }

    #[tokio::test]
    async fn signed_out_on_both_lists_is_no_session() {
        let walk = read_follows(|_, _| std::future::ready(FollowFetch::Unauthorized), &NO_PAUSE).await;
        assert_eq!(walk.end, WalkEnd::Unauthorized);
        assert!(imported_follows(walk).is_none());
    }

    #[tokio::test]
    async fn a_list_that_never_ends_stops_at_the_page_cap() {
        let mut calls = 0;
        let walk = read_follows(
            |_, cursor: Option<String>| {
                calls += 1;
                std::future::ready(kick_page(usize::MAX, 1, cursor.as_deref()))
            },
            &NO_PAUSE,
        )
        .await;
        assert_eq!(walk.end, WalkEnd::Capped);
        assert_eq!(calls, FOLLOW_PAGE_CAP);
        assert_eq!(walk.channels.len(), FOLLOW_PAGE_CAP);
    }

    #[test]
    fn rows_in_the_shapes_both_lists_send_are_read() {
        // The keys each list sent on 2026-10-29.
        let sidebar = json!({ "nextCursor": 5, "channels": [{
            "category_name": "Just Chatting", "channel_slug": "SomeStreamer", "is_live": true,
            "is_reserved": false, "profile_picture": "https://files.kick.com/a.webp",
            "session_title": "hi", "show_view_count": true, "user_username": "SomeStreamer",
            "viewer_count": 10
        }] });
        let page = json!({ "channels": [{
            "banner_picture": null, "category_name": null, "channel_slug": "another",
            "is_live": false, "profile_picture": null, "show_view_count": true,
            "user_username": "Another", "viewer_count": 0
        }] });
        let a = followed_channel(&follow_rows(&sidebar).unwrap()[0]).unwrap();
        assert_eq!((a.slug.as_str(), a.username.as_deref(), a.is_live), ("somestreamer", Some("SomeStreamer"), true));
        assert_eq!(a.profile_pic.as_deref(), Some("https://files.kick.com/a.webp"));
        let b = followed_channel(&follow_rows(&page).unwrap()[0]).unwrap();
        assert_eq!((b.slug.as_str(), b.username.as_deref(), b.profile_pic.as_deref(), b.is_live), ("another", Some("Another"), None, false));
        assert!(follow_rows(&json!({ "message": "Unauthenticated." })).is_none());
        assert_eq!(follow_rows(&json!([{ "slug": "x" }])).map(Vec::len), Some(1));
    }

    const NOW: &str = "2026-10-29T12:00:00Z";

    fn at(s: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&chrono::Utc)
    }

    /// A `new_subscriptions` row as kick.com's Subscriptions page reads it.
    fn sub(slug: &str, status: &str, expires_at: &str) -> serde_json::Value {
        json!({
            "channel": { "slug": slug, "username": slug.to_uppercase(), "profile_image": "https://files.kick.com/s.webp" },
            "status": status, "expires_at": expires_at, "provider": "stripe",
            "provider_id": "sub_1", "created_at": "2026-09-01T00:00:00.000000Z"
        })
    }

    #[test]
    fn current_subscriptions_are_read_from_the_list_kick_com_reads() {
        let body = json!({
            "new_subscriptions": [
                sub("active", "active", "2026-11-28T08:49:03.000000Z"),
                sub("canceled-running", "canceled", "2026-11-02T00:00:00.000000Z"),
                sub("canceled-over", "canceled", "2026-10-01T00:00:00.000000Z"),
                sub("expired", "expired", "2026-12-01T00:00:00.000000Z"),
                sub("active", "active", "2026-11-28T08:49:03.000000Z"),
            ],
            "subscriptions": [{ "channel": { "username": "Older_One" } }],
            "new_gifts": [sub("gifted", "active", "2026-11-28T00:00:00.000000Z")],
        });
        let subs = subscribed_channels(&body, at(NOW)).unwrap();
        let slugs: Vec<_> = subs.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(slugs, ["active", "canceled-running", "older_one"], "gifts and lapsed subs are left out");
        assert!(subs.iter().all(|c| c.subscribed && !c.is_live));
        assert_eq!(subs[0].username.as_deref(), Some("ACTIVE"));
        assert_eq!(subs[0].profile_pic.as_deref(), Some("https://files.kick.com/s.webp"));
    }

    #[test]
    fn a_reply_with_no_subscription_list_is_none_and_an_empty_one_is_empty() {
        assert!(subscribed_channels(&json!({ "message": "Unauthenticated." }), at(NOW)).is_none());
        let empty = json!({ "subscriptions": [], "new_subscriptions": [], "new_gifts": [] });
        assert_eq!(subscribed_channels(&empty, at(NOW)).map(|s| s.len()), Some(0));
        let v1 = json!([sub("v1", "active", "2027-01-01T00:00:00Z")]);
        assert_eq!(subscribed_channels(&v1, at(NOW)).map(|s| s.len()), Some(1));
    }

    #[tokio::test]
    async fn subscriptions_mark_follows_and_add_the_ones_not_followed() {
        let mut lists = Vec::new();
        let channels = read_account(
            |list, cursor: Option<String>| {
                lists.push(list);
                std::future::ready(match list {
                    PAGE => kick_page(3, 50, cursor.as_deref()),
                    _ => FollowFetch::Page(json!({
                        "new_subscriptions": [
                            sub("chan1", "active", "2026-11-28T00:00:00Z"),
                            sub("onlysubbed", "canceled", "2026-11-05T00:00:00Z"),
                        ],
                        "subscriptions": [], "new_gifts": []
                    })),
                })
            },
            &NO_PAUSE,
            at(NOW),
        )
        .await
        .expect("the follow list");
        let subbed: Vec<_> = channels.iter().filter(|c| c.subscribed).map(|c| c.slug.as_str()).collect();
        assert_eq!(subbed, ["chan1", "onlysubbed"]);
        assert_eq!(channels.len(), 4, "a channel subscribed to but not followed joins the list");
        assert_eq!(lists, [PAGE, "/api/v2/user/subscriptions"]);
    }

    #[tokio::test]
    async fn the_v1_list_is_read_only_when_v2_gives_none() {
        let channels = read_account(
            |list, cursor: Option<String>| {
                std::future::ready(match list {
                    PAGE => kick_page(2, 50, cursor.as_deref()),
                    "/api/v2/user/subscriptions" => FollowFetch::Page(json!({ "message": "moved" })),
                    _ => FollowFetch::Page(json!([sub("chan0", "active", "2027-01-01T00:00:00Z")])),
                })
            },
            &NO_PAUSE,
            at(NOW),
        )
        .await
        .expect("the follow list");
        assert!(channels[0].subscribed);
        assert!(!channels[1].subscribed);
    }

    #[tokio::test]
    async fn a_failed_subscription_read_never_costs_the_follow_list() {
        let channels = read_account(
            |list, cursor: Option<String>| {
                std::future::ready(match list {
                    PAGE => kick_page(3, 50, cursor.as_deref()),
                    _ => FollowFetch::Failed("HTTP 404".into()),
                })
            },
            &NO_PAUSE,
            at(NOW),
        )
        .await
        .expect("follows survive");
        assert_eq!(channels.len(), 3);
        assert!(channels.iter().all(|c| !c.subscribed));
    }

    #[tokio::test]
    async fn a_follow_list_that_fails_is_none_and_never_asks_for_subscriptions() {
        let mut lists = Vec::new();
        let result = read_account(
            |list, _| {
                lists.push(list);
                std::future::ready(FollowFetch::Failed("HTTP 404".into()))
            },
            &NO_PAUSE,
            at(NOW),
        )
        .await;
        assert!(result.is_none());
        assert_eq!(lists, [PAGE, SIDEBAR]);
    }
}
