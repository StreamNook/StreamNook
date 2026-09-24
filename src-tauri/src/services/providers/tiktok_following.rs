//! Which creators a signed-in TikTok account follows are live right now, for
//! the Following tab, the sidebar and go-live alerts.
//!
//! TikTok's own LIVE "Following" tab asks `/webcast/feed/` with
//! `channel_id=88` (its follow-feed channel) and
//! `req_from=live_mt_pc_web_follow_tab_refresh`. Like every feed request it is
//! signed inside TikTok's page, and it is answered for whoever the session
//! cookies belong to. So the directory's mint-and-replay applies with one
//! difference: the page that signs it is the SIGNED-IN one. A hidden window on
//! the sign-in profile opens the Following page once, the page makes its own
//! request, and Rust keeps that signed URL (per account) and replays it with the
//! account's session on the live poller's clock.
//!
//! The answer is the feed's usual shape, so it is read by the directory's own
//! parser, which also records each room's stream data: a click on a followed
//! creator plays without asking TikTok about the room first.

use crate::models::provider_stream::ProviderStream;
use crate::services::providers::tiktok_feed;
use anyhow::{anyhow, Result};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;

/// TikTok's `FollowFeedTab` channel.
const FOLLOW_FEED_CHANNEL: &str = "88";
const FOLLOWING_PAGE: &str = "https://www.tiktok.com/live/following";
/// The hidden window that signs. Named by no capability.
const LABEL: &str = "tiktok-following";
const REPLAY_TIMEOUT: Duration = Duration::from_secs(8);
/// After a signing that did not produce a request TikTok accepts, the next one
/// waits this long. Each loads TikTok's page in a hidden window, and the live
/// poller asks every 30 seconds.
const SIGN_RETRY: Duration = Duration::from_secs(600);

/// The account's signed Following request, replayable from here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SignedFollowing {
    /// Whose it is. A different account signed in means a new signing.
    account: String,
    /// The user agent the page signed under; the signature covers it.
    ua: String,
    url: String,
    /// Unix seconds, for reporting how long a signature lasted when refused.
    at: u64,
}

static SIGNED: Lazy<Mutex<Option<SignedFollowing>>> = Lazy::new(|| Mutex::new(load()));
static SIGNING: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));
/// When a signing last came to nothing, for `SIGN_RETRY`.
static SIGN_FAILED_AT: Lazy<Mutex<Option<std::time::Instant>>> = Lazy::new(|| Mutex::new(None));

/// The live creators the signed-in account follows.
pub async fn followed_live() -> Result<Vec<ProviderStream>> {
    let cookies = crate::services::tiktok_auth_service::cookie_header()
        .ok_or_else(|| anyhow!("not signed in to TikTok"))?;
    let account = crate::services::tiktok_auth_service::account_id().unwrap_or_default();

    if let Some(signed) = current(&account) {
        match replay(&signed, &cookies).await {
            Ok(rows) => return Ok(rows),
            Err(Replay::Refused(why)) => {
                let age = tiktok_feed::unix_now().saturating_sub(signed.at) / 60;
                log::info!(
                    "[TikTokFollowing] the signed request is no longer accepted ({}, signed {} min ago); signing afresh",
                    why,
                    age
                );
                drop_signed();
            }
            Err(Replay::Failed(e)) => return Err(e),
        }
    }
    let signed = sign(&account).await?;
    replay(&signed, &cookies).await.map_err(|e| match e {
        Replay::Refused(why) => {
            // Refused straight after signing: another signing now would load the
            // page again for the same answer.
            note_sign_failed();
            anyhow!("TikTok refused its own page's Following request: {}", why)
        }
        Replay::Failed(e) => e,
    })
}

/// Drop the signed request. Called on sign-out: it was made with that
/// account's session and device, and must not outlive it. The next account
/// signs at once, whatever happened to this one's signings.
pub fn forget() {
    drop_signed();
    if let Ok(mut f) = SIGN_FAILED_AT.lock() {
        *f = None;
    }
}

fn drop_signed() {
    if let Ok(mut s) = SIGNED.lock() {
        *s = None;
    }
    if let Some(p) = path() {
        let _ = std::fs::remove_file(p);
    }
}

fn note_sign_failed() {
    if let Ok(mut f) = SIGN_FAILED_AT.lock() {
        *f = Some(std::time::Instant::now());
    }
}

/// How long until a signing may be tried again, if one failed recently.
fn sign_retry_in() -> Option<Duration> {
    let failed = (*SIGN_FAILED_AT.lock().ok()?)?;
    SIGN_RETRY.checked_sub(failed.elapsed()).filter(|d| !d.is_zero())
}

fn current(account: &str) -> Option<SignedFollowing> {
    SIGNED
        .lock()
        .ok()?
        .clone()
        .filter(|s| s.account == account)
}

async fn sign(account: &str) -> Result<SignedFollowing> {
    let _one = SIGNING.lock().await;
    // Signed by another caller while this one waited.
    if let Some(s) = current(account) {
        return Ok(s);
    }
    if let Some(wait) = sign_retry_in() {
        return Err(anyhow!(
            "TikTok's Following page could not be signed recently; trying again in {} min",
            wait.as_secs().div_ceil(60)
        ));
    }
    let (ua, url) = tiktok_feed::capture_feed(
        LABEL,
        FOLLOWING_PAGE,
        crate::services::tiktok_auth_service::tiktok_profile_dir(),
        FOLLOW_FEED_CHANNEL,
    )
    .await
    .inspect_err(|_| note_sign_failed())?;
    let signed = SignedFollowing {
        account: account.to_string(),
        ua,
        url,
        at: tiktok_feed::unix_now(),
    };
    if let Ok(mut s) = SIGNED.lock() {
        *s = Some(signed.clone());
    }
    save(&signed);
    Ok(signed)
}

enum Replay {
    /// TikTok answered and would not serve it: sign afresh.
    Refused(String),
    /// Nothing TikTok said about the request: the network, or a server error.
    Failed(anyhow::Error),
}

async fn replay(signed: &SignedFollowing, cookies: &str) -> std::result::Result<Vec<ProviderStream>, Replay> {
    if !tiktok_feed::feed_url_ok(&signed.url) || !tiktok_feed::ua_ok(&signed.ua) {
        return Err(Replay::Refused("not a replayable feed request".into()));
    }
    let res = crate::services::http::client()
        .get(&signed.url)
        .header("User-Agent", &signed.ua)
        .header("Referer", FOLLOWING_PAGE)
        .header("Cookie", cookies)
        .timeout(REPLAY_TIMEOUT)
        .send()
        .await
        .map_err(|e| Replay::Failed(anyhow!("TikTok's Following list did not answer: {e}")))?;
    let status = res.status();
    if status.is_server_error() {
        return Err(Replay::Failed(anyhow!("TikTok's Following list answered HTTP {}", status.as_u16())));
    }
    if !status.is_success() {
        return Err(Replay::Refused(format!("HTTP {}", status.as_u16())));
    }
    let body = res
        .bytes()
        .await
        .map_err(|e| Replay::Failed(anyhow!("TikTok's Following list stopped answering: {e}")))?;
    tiktok_feed::read_feed(&body).map_err(Replay::Refused)
}

fn path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("tiktok_following_signed.json"))
}

fn load() -> Option<SignedFollowing> {
    let raw = std::fs::read(path()?).ok()?;
    let s: SignedFollowing = serde_json::from_slice(&raw).ok()?;
    (tiktok_feed::feed_url_ok(&s.url) && tiktok_feed::ua_ok(&s.ua)).then_some(s)
}

fn save(s: &SignedFollowing) {
    if let (Some(p), Ok(json)) = (path(), serde_json::to_vec(s)) {
        let _ = std::fs::write(p, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_request_belongs_to_one_account() {
        let s = SignedFollowing {
            account: "111".into(),
            ua: "UA".into(),
            url: "https://webcast.us.tiktok.com/webcast/feed/?channel_id=88".into(),
            at: 1,
        };
        if let Ok(mut g) = SIGNED.lock() {
            *g = Some(s.clone());
        }
        assert_eq!(current("111"), Some(s));
        assert_eq!(current("222"), None, "another account signs its own");
        if let Ok(mut g) = SIGNED.lock() {
            *g = None;
        }
    }

    #[test]
    fn a_failed_signing_waits_before_the_next() {
        note_sign_failed();
        let wait = sign_retry_in().expect("a retry is scheduled");
        assert!(wait > SIGN_RETRY - Duration::from_secs(5) && wait <= SIGN_RETRY);
        // What `forget` does to it, without touching the file on disk.
        if let Ok(mut f) = SIGN_FAILED_AT.lock() {
            *f = None;
        }
        assert_eq!(sign_retry_in(), None);
    }

    #[test]
    fn what_is_kept_on_disk_reads_back() {
        let s = SignedFollowing {
            account: "111".into(),
            ua: "UA".into(),
            url: "https://webcast.us.tiktok.com/webcast/feed/?channel_id=88".into(),
            at: 42,
        };
        let back: SignedFollowing = serde_json::from_slice(&serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
