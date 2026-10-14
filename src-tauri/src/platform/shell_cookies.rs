//! The cookie jar the iPhone app's shell fills.
//!
//! On iOS the core has no webview of its own. The Swift app's sign-in sheet
//! owns a `WKWebView` and hands the core every cookie its store holds for a
//! site, one snapshot per origin, through `ios_ffi::set_web_cookies`: at
//! launch, whenever the store changes, and after each page load.
//! `platform::cookies` reads this jar where the desktop reads a webview's, so
//! every service that harvests a site's cookies through `cookies_for_origin`
//! (the Twitch session today; Kick, 7TV and Drops as their sheets arrive)
//! reads the iPhone's sign-in with no iOS code of its own.
//!
//! Nothing here reaches the disk. WebKit's own store keeps the session across
//! launches and the shell pushes it again at the next start.

use super::cookies::CookieRow;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// One cookie as the shell sends it, with `WKHTTPCookieStore`'s fields.
/// `expires` is seconds since the Unix epoch, absent for a session cookie.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellCookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub expires: Option<f64>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
}

/// The shell's latest snapshot for each origin it reports.
#[derive(Default)]
struct Jar {
    by_origin: HashMap<String, Vec<ShellCookie>>,
}

impl Jar {
    /// Replaces everything held for `origin`, so a cookie the site deleted is
    /// gone here too. A snapshot that does not parse leaves the previous one.
    fn replace(&mut self, origin: &str, cookies_json: &str) -> Result<usize, String> {
        let cookies: Vec<ShellCookie> = serde_json::from_str(cookies_json)
            .map_err(|e| format!("the cookie snapshot is not a cookie list: {e}"))?;
        let held = cookies.len();
        self.by_origin.insert(origin_key(origin), cookies);
        Ok(held)
    }

    /// Every cookie still alive at `now` (epoch seconds), whatever origin
    /// reported it. A cookie two origins both report appears once, the most
    /// recently reported copy winning.
    fn rows(&self, now: f64) -> Vec<CookieRow> {
        let mut seen: HashMap<(String, String, String), CookieRow> = HashMap::new();
        for cookie in self.by_origin.values().flatten() {
            if cookie.expires.is_some_and(|at| at <= now) || cookie.value.is_empty() {
                continue;
            }
            seen.insert(
                (
                    cookie.name.clone(),
                    cookie.domain.to_ascii_lowercase(),
                    cookie.path.clone(),
                ),
                CookieRow {
                    name: cookie.name.clone(),
                    value: cookie.value.clone(),
                    domain: cookie.domain.clone(),
                    secure: cookie.secure,
                },
            );
        }
        seen.into_values().collect()
    }

    /// Drops every cookie that applies to any of these sites' hosts, from
    /// whichever origin reported it.
    fn forget(&mut self, urls: &[&str]) {
        let hosts: Vec<String> = urls
            .iter()
            .filter_map(|u| url::Url::parse(u).ok())
            .filter_map(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
            .collect();
        for cookies in self.by_origin.values_mut() {
            cookies.retain(|c| !hosts.iter().any(|h| super::cookies::domain_matches(&c.domain, h)));
        }
    }
}

/// `https://www.twitch.tv/` and `https://WWW.twitch.tv` are one origin.
fn origin_key(origin: &str) -> String {
    origin.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn jar() -> &'static Mutex<Jar> {
    static JAR: OnceLock<Mutex<Jar>> = OnceLock::new();
    JAR.get_or_init(Default::default)
}

/// Takes the shell's snapshot of `origin`'s cookies (a JSON list of
/// [`ShellCookie`]) and says how many are now held for it.
pub fn replace(origin: &str, cookies_json: &str) -> Result<usize, String> {
    jar()
        .lock()
        .map_err(|_| "the cookie jar is poisoned".to_string())?
        .replace(origin, cookies_json)
}

/// Every live cookie the shell handed over. `cookies_for_origin` picks a
/// site's out of these by domain, as an engine would.
pub fn rows() -> Vec<CookieRow> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    jar().lock().map(|jar| jar.rows(now)).unwrap_or_default()
}

/// Forgets the cookies of these sites until the shell pushes them again (a
/// sign-out that the shell clears from its store as well).
pub fn forget(urls: &[&str]) {
    if let Ok(mut jar) = jar().lock() {
        jar.forget(urls);
    }
}

#[cfg(test)]
mod tests {
    use super::Jar;

    const NOW: f64 = 1_800_000_000.0;

    fn names(jar: &Jar) -> Vec<String> {
        let mut names: Vec<String> = jar.rows(NOW).into_iter().map(|r| r.name).collect();
        names.sort();
        names
    }

    #[test]
    fn a_snapshot_replaces_what_its_origin_held() {
        let mut jar = Jar::default();
        jar.replace(
            "https://www.twitch.tv",
            r#"[{"name":"auth-token","value":"t","domain":".twitch.tv","path":"/","secure":true,"httpOnly":false},
                {"name":"unique_id","value":"d","domain":".twitch.tv","path":"/","expires":1900000000}]"#,
        )
        .unwrap();
        assert_eq!(names(&jar), ["auth-token", "unique_id"]);

        // Signing out on the site drops the token from the next snapshot.
        jar.replace(
            "https://www.twitch.tv/",
            r#"[{"name":"unique_id","value":"d","domain":".twitch.tv","path":"/"}]"#,
        )
        .unwrap();
        assert_eq!(names(&jar), ["unique_id"]);
    }

    #[test]
    fn origins_are_kept_apart_and_read_together() {
        let mut jar = Jar::default();
        jar.replace("https://www.twitch.tv", r#"[{"name":"auth-token","value":"t","domain":".twitch.tv"}]"#)
            .unwrap();
        jar.replace("https://kick.com", r#"[{"name":"session","value":"k","domain":"kick.com"}]"#)
            .unwrap();
        jar.replace("https://kick.com", "[]").unwrap();
        assert_eq!(names(&jar), ["auth-token"]);
    }

    #[test]
    fn expired_and_empty_cookies_are_not_served() {
        let mut jar = Jar::default();
        jar.replace(
            "https://www.twitch.tv",
            r#"[{"name":"old","value":"x","domain":".twitch.tv","expires":1700000000},
                {"name":"blank","value":"","domain":".twitch.tv"},
                {"name":"live","value":"y","domain":".twitch.tv","expires":1900000000}]"#,
        )
        .unwrap();
        assert_eq!(names(&jar), ["live"]);
    }

    #[test]
    fn a_snapshot_that_does_not_parse_keeps_the_last_good_one() {
        let mut jar = Jar::default();
        jar.replace("https://www.twitch.tv", r#"[{"name":"auth-token","value":"t","domain":".twitch.tv"}]"#)
            .unwrap();
        assert!(jar.replace("https://www.twitch.tv", "{not json").is_err());
        assert_eq!(names(&jar), ["auth-token"]);
    }

    #[test]
    fn forgetting_a_site_leaves_the_others_signed_in() {
        let mut jar = Jar::default();
        jar.replace("https://www.twitch.tv", r#"[{"name":"auth-token","value":"t","domain":".twitch.tv"}]"#)
            .unwrap();
        jar.replace(
            "https://kick.com",
            r#"[{"name":"session_token","value":"k","domain":"kick.com"},
                {"name":"XSRF","value":"x","domain":".id.kick.com"}]"#,
        )
        .unwrap();
        jar.forget(&["https://kick.com", "https://id.kick.com"]);
        assert_eq!(names(&jar), ["auth-token"]);
    }

    #[test]
    fn the_same_cookie_from_two_origins_is_served_once() {
        let mut jar = Jar::default();
        let token = r#"[{"name":"auth-token","value":"t","domain":".twitch.tv","path":"/"}]"#;
        jar.replace("https://www.twitch.tv", token).unwrap();
        jar.replace("https://twitch.tv", token).unwrap();
        assert_eq!(names(&jar), ["auth-token"]);
    }
}
