//! Twitch Client-Integrity tokens, minted in a hidden twitch.tv page.
//!
//! Some of Twitch's private GQL mutations (cheering with `sendCheer` among
//! them) refuse a request without a `Client-Integrity` header: the answer is
//! `IntegrityCheckFailed` and nothing happens. The token comes from
//! `gql.twitch.tv/integrity`, which sits behind Twitch's in-page bot check: the
//! check's script has to run in a real browser on a twitch.tv page and stamps
//! the integrity request itself. So this borrows a browser, the way
//! `youtube_potoken` does for YouTube.
//!
//! The page is `www.twitch.tv/robots.txt` (the lightest page on the origin: no
//! player, no chat, no app) in a hidden window on the signed-in twitch.tv web
//! profile, so the token is minted for the same account and device the cheer
//! goes out as. The script loads the bot check, asks for a token with the
//! session's own cookies, and reports back through the URL fragment, the only
//! channel out of the page. The window closes as soon as it answers.
//!
//! A token lives for hours, so one mint serves every cheer until it nears
//! expiry or Twitch refuses it; an account change forgets it.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID;

/// The page the token is minted on.
const MINT_PAGE: &str = "https://www.twitch.tv/robots.txt#SN_MINT";

/// Twitch's bot-check script, the one its own pages load.
const BOT_CHECK_SCRIPT: &str =
    "https://k.twitchcdn.net/149e9513-01fa-4fb0-aad4-566afd725d1b/2d206a39-8ed7-437e-a3be-862e0f06eea3/p.js";

/// The script measures in a few seconds; past this something is wrong.
const MINT_TIMEOUT: Duration = Duration::from_secs(30);

/// Re-mint this long before Twitch's own expiry.
const EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);

/// Never keep a token longer than this, whatever its expiry says.
const MAX_AGE: Duration = Duration::from_secs(8 * 3600);

/// What a call made with the token must carry: the token itself, and the
/// device id and browser it was minted for.
#[derive(Clone, Debug, PartialEq)]
pub struct Integrity {
    pub token: String,
    pub device_id: String,
    pub user_agent: String,
}

#[derive(Deserialize, Debug)]
struct Minted {
    token: String,
    /// Twitch's expiry, in milliseconds since the epoch (0 when unknown).
    #[serde(default)]
    expiration: i64,
    device_id: String,
    user_agent: String,
}

static CACHE: Mutex<Option<(Integrity, Instant)>> = Mutex::new(None);
/// When a mint last failed, so a warm-up does not open a page per keystroke
/// while the session cannot mint.
static LAST_FAILURE: Mutex<Option<Instant>> = Mutex::new(None);
const WARM_BACKOFF: Duration = Duration::from_secs(120);
static MINTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// When a mint last succeeded (ms since the epoch), for callers that back off.
static LAST_SUCCESS: Mutex<Option<i64>> = Mutex::new(None);
/// The phone side's own deadline: it always answers by then and removes its view.
/// Used by the phone arms only.
#[allow(dead_code)]
const NATIVE_DEADLINE_MS: u64 = 28_000;
/// The longest a phone mint can hold `MINTING`, whatever the phone side does.
#[allow(dead_code)]
const NATIVE_CEILING_MS: u64 = 35_000;

fn cached() -> Option<Integrity> {
    let guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (token, until) = guard.as_ref()?;
    (Instant::now() < *until).then(|| token.clone())
}

/// A token is cached and fresh, so a call needs no mint.
pub fn has_cached() -> bool {
    cached().is_some()
}

/// When a mint last succeeded, if ever this run.
pub fn last_success_ms() -> Option<i64> {
    *LAST_SUCCESS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The cached token, without minting (whisper_sync's minter reads it).
pub fn cached_token() -> Option<Integrity> {
    cached()
}

/// Forget the token: Twitch refused it, or the account changed.
pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// How long to keep a token Twitch says expires at `expiration_ms`.
fn keep_for(expiration_ms: i64, now_ms: i64) -> Duration {
    if expiration_ms <= 0 {
        return MAX_AGE;
    }
    let left = Duration::from_millis(expiration_ms.saturating_sub(now_ms).max(0) as u64);
    left.saturating_sub(EXPIRY_MARGIN).min(MAX_AGE)
}

/// A token for the signed-in twitch.tv session: the cached one, or a fresh
/// mint (one at a time; callers waiting share it).
pub async fn get() -> Result<Integrity, String> {
    if let Some(t) = cached() {
        return Ok(t);
    }
    let _one = MINTING.lock().await;
    if let Some(t) = cached() {
        return Ok(t);
    }
    let minted = match mint().await {
        Ok(m) => m,
        Err(e) => {
            *LAST_FAILURE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            return Err(e);
        }
    };
    *LAST_SUCCESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(chrono::Utc::now().timestamp_millis());
    let integrity = Integrity { token: minted.token, device_id: minted.device_id, user_agent: minted.user_agent };
    let keep = keep_for(minted.expiration, chrono::Utc::now().timestamp_millis());
    if !keep.is_zero() {
        *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((integrity.clone(), Instant::now() + keep));
    }
    Ok(integrity)
}

/// Mint ahead of need (a cheer waiting on the viewer's yes), so the send does
/// not wait on the page.
pub fn warm() {
    if cached().is_some() {
        return;
    }
    // A phone in the background never opens a page to mint.
    if crate::services::window_visibility::phone_idle() {
        return;
    }
    let failed_recently = LAST_FAILURE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|at| at.elapsed() < WARM_BACKOFF);
    if failed_recently || MINTING.try_lock().is_err() {
        return;
    }
    tauri::async_runtime::spawn(async {
        if let Err(e) = get().await {
            log::warn!("[Integrity] warm-up mint failed: {e}");
        }
    });
}

/// The script the hidden page runs. It acts only on the mint page, so a stray
/// navigation inside the window never reruns it.
fn script() -> String {
    format!(
        r#"(() => {{
  if (location.hostname !== 'www.twitch.tv' || location.hash !== '#SN_MINT') return;
  const report = (key, value) => {{ location.hash = key + '=' + encodeURIComponent(value); }};
  const cookie = (name) => {{
    const hit = document.cookie.split('; ').find((c) => c.startsWith(name + '='));
    return hit ? decodeURIComponent(hit.slice(name.length + 1)) : null;
  }};
  const start = () => {{
    const auth = cookie('auth-token');
    const device = cookie('unique_id');
    if (!auth || !device) {{ report('SN_INTEGRITY_ERR', 'not signed in to twitch.tv'); return; }}
    const timer = setTimeout(() => report('SN_INTEGRITY_ERR', 'the bot check did not start'), 25000);
    const fail = (e) => {{ clearTimeout(timer); report('SN_INTEGRITY_ERR', String((e && e.message) || e)); }};
    document.addEventListener('kpsdk-load', () => {{
      window.KPSDK.configure([{{ protocol: 'https:', method: 'POST', domain: 'gql.twitch.tv', path: '/integrity' }}]);
    }}, {{ once: true }});
    document.addEventListener('kpsdk-ready', async () => {{
      try {{
        const resp = await window.fetch('https://gql.twitch.tv/integrity', {{
          method: 'POST', mode: 'cors', credentials: 'omit', body: null,
          headers: {{ 'Client-Id': '{client_id}', 'Authorization': 'OAuth ' + auth, 'X-Device-Id': device }},
        }});
        if (resp.status !== 200) throw new Error('integrity answered ' + resp.status);
        const json = await resp.json();
        if (!json || !json.token) throw new Error('no token in the answer');
        clearTimeout(timer);
        report('SN_INTEGRITY', JSON.stringify({{ token: json.token, expiration: json.expiration || 0, device_id: device, user_agent: navigator.userAgent }}));
      }} catch (e) {{ fail(e); }}
    }}, {{ once: true }});
    const s = document.createElement('script');
    s.src = '{script}';
    s.onerror = () => fail('the bot check script did not load');
    (document.body || document.documentElement).appendChild(s);
  }};
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start, {{ once: true }});
  else start();
}})();"#,
        client_id = TWITCH_WEB_CLIENT_ID,
        script = BOT_CHECK_SCRIPT,
    )
}

/// The page's answer in the window's URL fragment, once it has one.
fn read_fragment(url: &str) -> Option<Result<Minted, String>> {
    let value = |key: &str| {
        url.split(&format!("#{key}=")).nth(1).map(|rest| {
            let raw = rest.split('&').next().unwrap_or("");
            urlencoding::decode(raw).map(|s| s.into_owned()).unwrap_or_else(|_| raw.to_string())
        })
    };
    if let Some(json) = value("SN_INTEGRITY") {
        return Some(serde_json::from_str::<Minted>(&json).map_err(|e| format!("unreadable answer: {e}")));
    }
    value("SN_INTEGRITY_ERR").map(Err)
}

#[cfg(desktop)]
async fn mint() -> Result<Minted, String> {
    use crate::rt::WebviewWindowBuilder;
    use tauri::WebviewUrl;

    let app = crate::services::providers::app_handle().ok_or("no app handle; cannot open the twitch.tv page")?;
    let profile = crate::services::twitch_service::active_twitch_web_profile_dir().map_err(|e| e.to_string())?;
    let label = format!("twitch-integrity-{}", chrono::Utc::now().timestamp_millis());
    let url = MINT_PAGE.parse().map_err(|e| format!("bad mint page: {e}"))?;
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(url))
        .title("Twitch")
        .inner_size(480.0, 320.0)
        .visible(false)
        .skip_taskbar(true)
        .data_directory(profile)
        .initialization_script(script())
        .build()
        .map_err(|e| format!("could not open the twitch.tv page: {e}"))?;

    let started = Instant::now();
    let mut outcome = None;
    while started.elapsed() < MINT_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(u) = crate::platform::webview_url::current_url(&window).await {
            if let Some(r) = read_fragment(u.as_str()) {
                outcome = Some(r);
                break;
            }
        }
    }
    let _ = window.close();
    match outcome {
        Some(Ok(m)) => {
            log::info!("[Integrity] minted in {:.1}s", started.elapsed().as_secs_f64());
            Ok(m)
        }
        Some(Err(e)) => Err(format!("Twitch's security check failed: {e}")),
        None => Err(format!("Twitch's security check did not answer within {}s", MINT_TIMEOUT.as_secs())),
    }
}

/// The phone answers the minted page's final address (its fragment carries the
/// answer), read the same way the desktop window's is.
#[allow(dead_code)]
fn minted_from_url(url: &str) -> Result<Minted, String> {
    match read_fragment(url) {
        Some(Ok(m)) => Ok(m),
        Some(Err(e)) => Err(format!("Twitch's security check failed: {e}")),
        None => Err("Twitch's security check gave no answer".into()),
    }
}

/// On iPhone the shell runs the page in a 1 pt WKWebView on the sign-in's
/// store (`host_request` kind `mint_twitch_integrity`) and answers through
/// `twitch_integrity_minted`. `MINTING` is held at most `NATIVE_CEILING_MS`.
#[cfg(target_os = "ios")]
async fn mint() -> Result<Minted, String> {
    if crate::services::window_visibility::phone_idle() {
        return Err("Twitch's security check waits for the app to be open".into());
    }
    let (id, rx) = matcher::begin(NATIVE_CEILING_MS);
    sn_tauri_headless::host_request(
        "mint_twitch_integrity",
        &serde_json::json!({
            "request_id": id,
            "url": MINT_PAGE,
            "script": script(),
            "deadline_ms": NATIVE_DEADLINE_MS,
        }),
    );
    let url = matcher::wait(&id, rx, NATIVE_CEILING_MS).await?;
    minted_from_url(&url)
}

/// On Android the login plugin runs the page in a 1 px WebView on the app's
/// cookie jar (`mintIntegrity`), the browser the twitch.tv session was signed in
/// with, and answers with the page's final address. The Kotlin side always
/// answers by `NATIVE_DEADLINE_MS` and removes its view; the wait here is capped
/// at `NATIVE_CEILING_MS` whatever happens, so `MINTING` is never held longer.
#[cfg(target_os = "android")]
async fn mint() -> Result<Minted, String> {
    if crate::services::window_visibility::phone_idle() {
        return Err("Twitch's security check waits for the app to be open".into());
    }
    let app = crate::services::providers::app_handle().ok_or("no app handle; cannot open the twitch.tv page")?;
    let script = script();
    let job = tauri::async_runtime::spawn_blocking(move || {
        crate::twitch_login_plugin::mint_integrity(&app, MINT_PAGE, &script, NATIVE_DEADLINE_MS)
    });
    let url = match tokio::time::timeout(Duration::from_millis(NATIVE_CEILING_MS), job).await {
        Ok(Ok(answer)) => answer?,
        Ok(Err(e)) => return Err(format!("Twitch's security check could not run: {e}")),
        Err(_) => {
            return Err(format!(
                "Twitch's security check did not answer within {}s",
                NATIVE_CEILING_MS / 1000
            ))
        }
    };
    let minted = minted_from_url(&url)?;
    // The proof line S1 reads back: kept at a level release logs keep.
    log::warn!("[Integrity] minted on the phone");
    Ok(minted)
}

/// Requests a phone shell answers later, matched by id. Plain Rust with no
/// platform gate, so its rules are tested on desktop.
pub mod matcher {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use once_cell::sync::Lazy;
    use tokio::sync::oneshot;

    type Answer = Result<String, String>;

    static PENDING: Lazy<Mutex<HashMap<String, (oneshot::Sender<Answer>, Instant)>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    /// Open a request that may be answered within `ceiling_ms`.
    pub fn begin(ceiling_ms: u64) -> (String, oneshot::Receiver<Answer>) {
        let id = format!("mint-{}-{:08x}", chrono::Utc::now().timestamp_millis(), rand::random::<u32>());
        let (tx, rx) = oneshot::channel();
        let deadline = Instant::now() + Duration::from_millis(ceiling_ms);
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), (tx, deadline));
        (id, rx)
    }

    /// The shell's answer. False when the request is unknown, already
    /// answered, past its ceiling, or its waiter is gone.
    pub fn answer(request_id: &str, outcome: Answer) -> bool {
        let entry = PENDING.lock().unwrap_or_else(|e| e.into_inner()).remove(request_id);
        match entry {
            Some((tx, deadline)) if Instant::now() <= deadline => tx.send(outcome).is_ok(),
            _ => false,
        }
    }

    /// Drop a request nobody waits for any more; a late answer is then refused.
    pub fn forget(request_id: &str) {
        PENDING.lock().unwrap_or_else(|e| e.into_inner()).remove(request_id);
    }

    /// Wait for the answer at most `ceiling_ms`.
    pub async fn wait(request_id: &str, rx: oneshot::Receiver<Answer>, ceiling_ms: u64) -> Answer {
        match tokio::time::timeout(Duration::from_millis(ceiling_ms), rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => {
                forget(request_id);
                Err("Twitch's security check was cancelled".into())
            }
            Err(_) => {
                forget(request_id);
                Err(format!("Twitch's security check did not answer within {}s", ceiling_ms / 1000))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_minted_token_out_of_the_fragment() {
        let json = r#"{"token":"v4.public.abc","expiration":1700000000000,"device_id":"dev1","user_agent":"UA 1"}"#;
        let url = format!("https://www.twitch.tv/robots.txt#SN_INTEGRITY={}", urlencoding::encode(json));
        let m = read_fragment(&url).unwrap().unwrap();
        assert_eq!(m.token, "v4.public.abc");
        assert_eq!(m.device_id, "dev1");
        assert_eq!(m.user_agent, "UA 1");
        assert_eq!(m.expiration, 1_700_000_000_000);
    }

    #[test]
    fn reads_an_error_and_ignores_the_mint_marker() {
        let r = read_fragment("https://www.twitch.tv/robots.txt#SN_INTEGRITY_ERR=not%20signed%20in").unwrap();
        assert_eq!(r.unwrap_err(), "not signed in");
        assert!(read_fragment("https://www.twitch.tv/robots.txt#SN_MINT").is_none());
    }

    #[test]
    fn keeps_a_token_until_just_before_it_expires() {
        let now = 1_000_000_000_000;
        assert_eq!(keep_for(now + 3_600_000, now), Duration::from_secs(3600) - EXPIRY_MARGIN);
        assert_eq!(keep_for(now + 100 * 3_600_000, now), MAX_AGE);
        assert_eq!(keep_for(now + 60_000, now), Duration::ZERO);
        assert_eq!(keep_for(0, now), MAX_AGE);
    }

    #[test]
    fn the_script_only_runs_on_the_mint_page_and_names_the_web_client() {
        let s = script();
        assert!(s.contains("location.hash !== '#SN_MINT'"));
        assert!(s.contains(TWITCH_WEB_CLIENT_ID));
        assert!(s.contains(BOT_CHECK_SCRIPT));
    }

    #[tokio::test]
    async fn mint_matcher_resolves_and_times_out() {
        let (id, rx) = matcher::begin(5_000);
        assert!(matcher::answer(&id, Ok("https://www.twitch.tv/robots.txt#x".into())));
        assert_eq!(matcher::wait(&id, rx, 5_000).await.unwrap(), "https://www.twitch.tv/robots.txt#x");
        // Answered once: a second answer is refused.
        assert!(!matcher::answer(&id, Ok("again".into())));
        assert!(!matcher::answer("mint-unknown", Ok("x".into())));
    }

    #[tokio::test]
    async fn late_native_answer_after_timeout_is_dropped() {
        let (id, rx) = matcher::begin(5_000);
        let r = matcher::wait(&id, rx, 20).await;
        assert!(r.unwrap_err().contains("did not answer"));
        assert!(!matcher::answer(&id, Ok("late".into())));
    }

    #[tokio::test]
    async fn native_never_answers_releases_minting() {
        // The wait ends by itself, so a mint that holds MINTING returns and
        // releases it even when the phone side never answers.
        let lock = tokio::sync::Mutex::new(());
        {
            let _held = lock.lock().await;
            let (id, rx) = matcher::begin(30);
            assert!(matcher::wait(&id, rx, 30).await.is_err());
        }
        assert!(lock.try_lock().is_ok());
    }

    #[test]
    fn a_minted_page_address_reads_like_the_desktop_one() {
        let url = format!(
            "https://www.twitch.tv/robots.txt#SN_INTEGRITY={}",
            urlencoding::encode(r#"{"token":"t","expiration":0,"device_id":"d","user_agent":"u"}"#)
        );
        assert_eq!(minted_from_url(&url).unwrap().token, "t");
        assert!(minted_from_url("https://www.twitch.tv/robots.txt").is_err());
    }
}
