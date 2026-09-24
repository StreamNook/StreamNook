//! TikTok sign-in, for one purpose: playing LIVE rooms TikTok keeps behind an
//! age check.
//!
//! Same shape as the YouTube session and for the same reason: there is no
//! OAuth app to register and no client secret, so nothing needs to be baked
//! into the build. The user signs in to tiktok.com normally, in the shared
//! login overlay, inside a persistent per-platform WebView2 profile; Rust then
//! reads that profile's cookie jar (which sees the HttpOnly `sessionid` page
//! script never can) and keeps the whole tiktok.com cookie set in the keyring,
//! with an obfuscated-file fallback, exactly as the YouTube session is kept.
//!
//! WHERE THE SESSION IS USED, AND WHERE IT IS NOT. Two requests carry it:
//!   * room info on the watch path, only after TikTok has refused that same
//!     request anonymously as age restricted;
//!   * the account's Following LIVE list (`providers::tiktok_following`), on
//!     the live poller's clock while signed in, so the Following tab, the
//!     sidebar and go-live alerts work the way they do on every other platform.
//!     That is background traffic attributable to the account, accepted
//!     deliberately for parity: one request per poll, the same one TikTok's own
//!     Following tab makes.
//!
//! Everything else stays anonymous: the per-creator liveness sweep of follows
//! made in StreamNook, the Top live directory, profile reads and chat, which
//! StreamNook already declines to send on TikTok. Do not thread the cookies
//! into anything beyond these two.

use crate::services::twitch_service::get_app_data_dir;
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const ORIGIN: &str = "https://www.tiktok.com";
const LOGIN_WINDOW_LABEL: &str = "tiktok-login";
const LOGIN_URL: &str = "https://www.tiktok.com/login";
/// Who the session belongs to, or a definitive "signed out". Answers without
/// the page's request signing, which the LIVE endpoints demand.
const ACCOUNT_INFO_URL: &str =
    "https://www.tiktok.com/passport/web/account/info/?aid=1459&app_language=en&device_platform=web_pc";
const KEYRING_SERVICE: &str = "streamnook_tiktok_session";
const KEYRING_USER: &str = "default";
const OBF_KEY: &[u8] = b"StreamNookTikTokKey2026";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct TikTokSession {
    /// The entire tiktok.com cookie set, not a chosen few: TikTok's edge
    /// checks more than `sessionid`, and sending what the browser sends is
    /// what makes the request look like the browser's.
    cookies: HashMap<String, String>,
    #[serde(default)]
    account_name: Option<String>,
    #[serde(default)]
    account_avatar: Option<String>,
    /// The numeric user id, which is also what TikTok chat stamps on the
    /// account's own messages.
    #[serde(default)]
    account_id: Option<String>,
    /// The @handle. Absent on a session stored before it was recorded, which
    /// is what tells `account_identity` to look the account up once more.
    #[serde(default)]
    account_handle: Option<String>,
}

static SESSION: OnceLock<Mutex<Option<TikTokSession>>> = OnceLock::new();

fn session_cell() -> &'static Mutex<Option<TikTokSession>> {
    SESSION.get_or_init(|| Mutex::new(load_persisted()))
}

fn session_path() -> Option<PathBuf> {
    get_app_data_dir().ok().map(|d| d.join(".tiktok_session"))
}

fn obfuscate(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ OBF_KEY[i % OBF_KEY.len()])
        .collect()
}

fn persist(sess: &TikTokSession) {
    let Ok(json) = serde_json::to_string(sess) else {
        return;
    };
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.set_password(&json);
    }
    if let Some(p) = session_path() {
        let _ = std::fs::write(p, obfuscate(json.as_bytes()));
    }
}

fn load_persisted() -> Option<TikTokSession> {
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(json) = entry.get_password() {
            if let Ok(s) = serde_json::from_str::<TikTokSession>(&json) {
                return Some(s);
            }
        }
    }
    let raw = std::fs::read(session_path()?).ok()?;
    let json = String::from_utf8(obfuscate(&raw)).ok()?;
    serde_json::from_str(&json).ok()
}

fn clear_persisted() {
    if let Ok(entry) = crate::services::secure_store::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
    if let Some(p) = session_path() {
        let _ = std::fs::remove_file(p);
    }
}

fn snapshot() -> Option<TikTokSession> {
    session_cell().lock().ok().and_then(|s| s.clone())
}

/// The WebView2 profile the sign-in overlay mounts into. Separate from the
/// directory page's profile, which stays signed out.
pub fn tiktok_profile_dir() -> PathBuf {
    let base = get_app_data_dir().unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("platform_web_profiles").join("tiktok");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The session cookie's value, or None when signed out.
fn session_id(cookies: &HashMap<String, String>) -> Option<&str> {
    cookies.get("sessionid").map(String::as_str).filter(|v| !v.is_empty())
}

fn signed_in(cookies: &HashMap<String, String>) -> bool {
    session_id(cookies).is_some()
}

/// A signed-in jar whose session is not the one left in the profile from
/// before this sign-in began.
fn is_new_session(cookies: &HashMap<String, String>, stale: Option<&str>) -> bool {
    crate::services::sign_in_profile::is_new_session(session_id(cookies), stale)
}

/// The sign-in profile, for signing it out and for starting a fresh sign-in.
#[cfg(desktop)]
fn profile() -> crate::services::sign_in_profile::SignInProfile {
    crate::services::sign_in_profile::SignInProfile {
        dir: tiktok_profile_dir(),
        overlay_label: LOGIN_WINDOW_LABEL,
        sign_out_label: "tiktok-sign-out",
        origin: ORIGIN,
        sites: &["tiktok.com"],
        session: session_id,
        tag: "tiktok",
    }
}

// --- Public surface ---------------------------------------------------------

/// Whether we hold a TikTok session. A pure read: it never notices a session
/// TikTok has revoked, which is `validate_session`'s job.
pub fn is_connected() -> bool {
    snapshot().map(|s| signed_in(&s.cookies)).unwrap_or(false)
}

/// The session as a `Cookie` header value, for the one request that uses it
/// (see the module comment before adding a second).
pub fn cookie_header() -> Option<String> {
    let s = snapshot()?;
    if !signed_in(&s.cookies) {
        return None;
    }
    let mut pairs: Vec<(&String, &String)> = s.cookies.iter().collect();
    pairs.sort();
    Some(
        pairs
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// The signed-in account's id (numeric), or None.
pub fn account_id() -> Option<String> {
    snapshot().and_then(|s| s.account_id)
}

/// Name and picture for the Accounts row, asking TikTok once if a session has
/// neither yet, or was stored before its @handle was recorded (and so may be
/// wearing the login system's name rather than the profile's).
pub async fn account_identity() -> (Option<String>, Option<String>) {
    let Some(s) = snapshot() else {
        return (None, None);
    };
    if (s.account_name.is_none() || s.account_handle.is_none()) && signed_in(&s.cookies) {
        if let Verdict::SignedIn(id) = identify().await {
            remember(&id);
            return (id.name, id.avatar);
        }
    }
    (s.account_name, s.account_avatar)
}

/// The signed-in account's @handle, or None.
pub fn account_handle() -> Option<String> {
    snapshot().and_then(|s| s.account_handle)
}

/// Sign out, leaving nothing of the account behind: not in memory, not in the
/// keyring or on disk, and not in the sign-in profile. That last one matters
/// beyond tidiness. A profile still signed in makes TikTok's page treat the
/// next "Continue with Apple" as LINKING Apple to the account already there, so
/// signing out and back in with Apple would bind the user's Apple ID to the
/// account they had just signed out of.
pub async fn disconnect() {
    if let Ok(mut s) = session_cell().lock() {
        *s = None;
    }
    clear_persisted();
    // The Following request was signed with this account's session and device.
    crate::services::providers::tiktok_following::forget();
    crate::services::providers::tiktok_send::forget();
    crate::services::provider_live_service::forget_provider("tiktok").await;
    // Through a live webview on the profile: deleting its folder does not
    // reliably sign it out. On macOS only TikTok's cookies go, since the
    // sign-in shares its store with the rest of the app there (see
    // `sign_in_profile`).
    #[cfg(desktop)]
    crate::services::sign_in_profile::sign_out(&profile()).await;
    crate::services::providers::emit_platform_account_changed(&["tiktok"]);
}

/// Ask TikTok whether the stored session is still accepted.
///
/// - `Some(true)`: TikTok named the account.
/// - `Some(false)`: TikTok said the session is over; it has been cleared.
/// - `None`: could not tell (offline, reshaped payload). Nothing changes, so a
///   network blip or a TikTok redesign never signs anyone out.
pub async fn validate_session() -> Option<bool> {
    if !is_connected() {
        return None;
    }
    match probe().await {
        Verdict::SignedIn(id) => {
            // The account id only. Name, picture and handle come from
            // `identify`, and this answer's own name and picture would put the
            // login system's record back over the profile's on every check.
            remember(&Identity {
                id: id.id,
                ..Default::default()
            });
            Some(true)
        }
        Verdict::SignedOut => {
            log::info!("[tiktok] stored session was rejected; signing out");
            disconnect().await;
            crate::services::providers::emit_platform_session_expired("tiktok");
            Some(false)
        }
        Verdict::Unknown => None,
    }
}

// --- Identity ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
struct Identity {
    name: Option<String>,
    avatar: Option<String>,
    id: Option<String>,
    handle: Option<String>,
}

#[derive(Debug, PartialEq)]
enum Verdict {
    SignedIn(Identity),
    SignedOut,
    Unknown,
}

async fn probe() -> Verdict {
    let Some(cookies) = cookie_header() else {
        return Verdict::SignedOut;
    };
    let res = crate::services::http::client()
        .get(ACCOUNT_INFO_URL)
        .header("User-Agent", UA)
        .header("Referer", "https://www.tiktok.com/")
        .header("Cookie", cookies)
        .timeout(Duration::from_secs(10))
        .send()
        .await;
    let Ok(res) = res else {
        return Verdict::Unknown;
    };
    if !res.status().is_success() {
        log::debug!("[tiktok] session check inconclusive: HTTP {}", res.status());
        return Verdict::Unknown;
    }
    match res.text().await {
        Ok(body) => read_account_info(&body),
        Err(_) => Verdict::Unknown,
    }
}

/// Read the account-info answer.
///
/// Signed out is recognised only by TikTok's own words for it, measured: HTTP
/// 200 with `"name":"session_expired"` and `"error_code":13`, for a missing
/// session and for a forged one alike. Anything else that is not a named
/// account is `Unknown`, never a sign-out.
fn read_account_info(body: &str) -> Verdict {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return Verdict::Unknown;
    };
    let data = &v["data"];
    if data["name"].as_str() == Some("session_expired") || data["error_code"].as_i64() == Some(13) {
        return Verdict::SignedOut;
    }
    if v["message"].as_str() != Some("success") {
        return Verdict::Unknown;
    }
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| data[*k].as_str().map(str::trim).filter(|s| !s.is_empty()))
            .map(str::to_string)
    };
    let id = text(&["user_id_str"]).or_else(|| data["user_id"].as_u64().map(|n| n.to_string()));
    if id.is_none() {
        return Verdict::Unknown;
    }
    Verdict::SignedIn(Identity {
        name: text(&["screen_name", "nickname", "username", "unique_id"]),
        avatar: text(&["avatar_url", "avatar_large_url", "avatar_thumb_url"])
            .filter(|u| u.starts_with("https://")),
        id,
        handle: text(&["username", "unique_id"]).filter(|h| is_handle(h)),
    })
}

fn is_handle(h: &str) -> bool {
    !h.is_empty() && h.len() <= 64 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Who the session is, as people see them. The account-info answer is the
/// authority on WHICH account it is (its id and @handle), but its
/// `screen_name` and picture are the login system's own record, which can
/// differ from the profile entirely (a name generated when a sign-in method
/// was linked, say). The public profile supplies the nickname and picture
/// everyone else sees. That lookup is anonymous, like every other profile read.
async fn identify() -> Verdict {
    match probe().await {
        Verdict::SignedIn(mut id) => {
            if let Some(handle) = id.handle.clone() {
                match crate::services::providers::tiktok_media::public_identity(&handle).await {
                    Ok((name, avatar)) => {
                        let name = name.trim();
                        if !name.is_empty() {
                            id.name = Some(name.to_string());
                        }
                        if avatar.starts_with("https://") {
                            id.avatar = Some(avatar);
                        }
                    }
                    Err(e) => {
                        log::debug!("[tiktok] profile for @{} unavailable: {}", handle, e);
                        // Left unrecorded, so the next read looks again rather
                        // than keeping the login system's name for good.
                        id.handle = None;
                    }
                }
            }
            Verdict::SignedIn(id)
        }
        other => other,
    }
}

fn remember(id: &Identity) {
    let mut updated = None;
    if let Ok(mut s) = session_cell().lock() {
        if let Some(sess) = s.as_mut() {
            sess.account_name = id.name.clone().or(sess.account_name.take());
            sess.account_avatar = id.avatar.clone().or(sess.account_avatar.take());
            sess.account_id = id.id.clone().or(sess.account_id.take());
            sess.account_handle = id.handle.clone().or(sess.account_handle.take());
            updated = Some(sess.clone());
        }
    }
    if let Some(sess) = updated {
        persist(&sess);
    }
}

// --- Connect ----------------------------------------------------------------

/// Sign in to TikTok in the shared login overlay, the same in-app surface
/// Twitch, Kick and YouTube use, then read the session off its profile.
#[cfg(desktop)]
pub async fn connect() -> Result<()> {
    use crate::services::sign_in_profile;
    use tauri::Manager;

    let app = crate::services::providers::app_handle()
        .ok_or_else(|| anyhow!("app handle not available for TikTok sign-in"))?;

    // Signing in while disconnected is a fresh sign-in, so whatever session the
    // profile still holds belongs to an account that was disconnected. The
    // overlay opens blank and shows the login page only once that session is
    // gone; otherwise the page opens signed in as the old account and the loop
    // below takes it (see `sign_in_profile`).
    let fresh = !is_connected();
    crate::commands::twitch::emit_overlay_open_with(
        &app,
        LOGIN_WINDOW_LABEL,
        sign_in_profile::opening_url(fresh, LOGIN_URL),
        "fullbody",
        Some("tiktok-account"),
    )
    .map_err(|e| anyhow!("couldn't open the TikTok sign-in: {}", e))?;

    // Rust asks, React measures, Rust builds: the window exists a moment later.
    let mut mounted = false;
    for _ in 0..60 {
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_some() {
            mounted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if !mounted {
        crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_WINDOW_LABEL);
        return Err(anyhow!("the TikTok sign-in never opened"));
    }

    let cookies_now = || {
        crate::services::youtube_auth_service::fetch_cookies_for_origin(
            &app,
            LOGIN_WINDOW_LABEL,
            &[],
            ORIGIN,
        )
    };

    // Belt and braces: the session the profile held when this began is never
    // mistaken for this sign-in, whatever the clear managed.
    let mut stale: Option<String> = None;
    if fresh {
        if let Some(win) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
            stale = sign_in_profile::begin_fresh(&app, &win, &profile(), LOGIN_URL).await;
        }
    }

    // `sessionid` lands on tiktok.com the moment sign-in completes, whichever
    // way the user signed in (QR code, password, Apple or Google).
    let mut harvested: Option<HashMap<String, String>> = None;
    let mut dismissed = false;
    for _ in 0..200 {
        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            dismissed = true;
            break;
        }
        if let Ok(map) = cookies_now().await {
            if is_new_session(&map, stale.as_deref()) {
                harvested = Some(map);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }

    crate::commands::twitch::dismiss_login_overlay(&app, LOGIN_WINDOW_LABEL);

    if dismissed && harvested.is_none() {
        log::info!("[tiktok] sign-in overlay dismissed by the user");
        return Err(anyhow!("Sign-in was cancelled"));
    }
    let cookies = harvested.ok_or_else(|| anyhow!("TikTok sign-in wasn't completed"))?;
    if let Ok(mut s) = session_cell().lock() {
        *s = Some(TikTokSession {
            cookies,
            ..Default::default()
        });
    }
    if let Some(sess) = snapshot() {
        persist(&sess);
    }
    // Who signed in, for the Accounts row. Best effort: a session TikTok
    // accepts but whose account answer we cannot read still plays.
    if let Verdict::SignedIn(id) = identify().await {
        remember(&id);
    }
    crate::services::providers::emit_platform_account_changed(&["tiktok"]);
    Ok(())
}

#[cfg(mobile)]
pub async fn connect() -> Result<()> {
    Err(anyhow!("TikTok sign-in is only available in the desktop app so far"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_out_is_recognised_by_tiktoks_own_answer() {
        // Verbatim, for no session and for a forged `sessionid` alike.
        let body = r#"{"data":{"description":"session expired, please sign in again","error_code":13,"name":"session_expired"},"message":"error"}"#;
        assert_eq!(read_account_info(body), Verdict::SignedOut);
    }

    #[test]
    fn anything_unrecognised_is_unknown_never_a_sign_out() {
        for body in [
            "",
            "<html>rate limited</html>",
            r#"{"message":"error","data":{"error_code":7,"name":"something_else"}}"#,
            r#"{"message":"success","data":{}}"#,
        ] {
            assert_eq!(read_account_info(body), Verdict::Unknown, "{body}");
        }
    }

    // The fields the reader accepts for a signed-in account. Not a captured
    // payload (this build has never been signed in); what it pins is that a
    // named account is read and that an unsafe picture url is not.
    #[test]
    fn a_named_account_is_read() {
        let body = r#"{"message":"success","data":{"user_id":7207281585031922730,"user_id_str":"7207281585031922730","screen_name":"Someone","username":"someone","avatar_url":"https://p16-sign.tiktokcdn-us.com/a.jpeg"}}"#;
        let Verdict::SignedIn(id) = read_account_info(body) else {
            panic!("a named account")
        };
        assert_eq!(id.id.as_deref(), Some("7207281585031922730"));
        assert_eq!(id.name.as_deref(), Some("Someone"));
        assert!(id.avatar.is_some());
        // The handle is what names the account unambiguously; the screen name is
        // the login system's and may not be what the profile shows.
        assert_eq!(id.handle.as_deref(), Some("someone"));

        let odd = r#"{"message":"success","data":{"user_id_str":"1","screen_name":"X","username":"not a handle"}}"#;
        let Verdict::SignedIn(id) = read_account_info(odd) else {
            panic!("a named account")
        };
        assert_eq!(id.handle, None, "only a real handle is kept");

        let plain = r#"{"message":"success","data":{"user_id":42,"username":"handle","avatar_url":"http://x/a.jpeg"}}"#;
        let Verdict::SignedIn(id) = read_account_info(plain) else {
            panic!("a named account")
        };
        assert_eq!(id.id.as_deref(), Some("42"));
        assert_eq!(id.name.as_deref(), Some("handle"), "falls back to the handle");
        assert!(id.avatar.is_none(), "plain http is not shown");
    }

    #[test]
    fn a_session_is_sessionid_and_nothing_less() {
        let mut jar = HashMap::new();
        jar.insert("ttwid".to_string(), "1|abc".to_string());
        jar.insert("msToken".to_string(), "x".to_string());
        assert!(!signed_in(&jar), "a signed-out browser has cookies too");
        jar.insert("sessionid".to_string(), String::new());
        assert!(!signed_in(&jar));
        jar.insert("sessionid".to_string(), "deadbeef".to_string());
        assert!(signed_in(&jar));
    }

    #[test]
    fn a_disconnected_accounts_session_is_never_taken_for_a_new_sign_in() {
        let mut jar = HashMap::new();
        jar.insert("sessionid".to_string(), "old-account".to_string());
        assert!(!is_new_session(&jar, Some("old-account")), "the leftover session");
        assert!(is_new_session(&jar, None), "a clean profile takes any session");
        jar.insert("sessionid".to_string(), "real-account".to_string());
        assert!(is_new_session(&jar, Some("old-account")), "signing in replaced it");
        jar.insert("sessionid".to_string(), String::new());
        assert!(!is_new_session(&jar, Some("old-account")), "signed out is not new");
    }
}
