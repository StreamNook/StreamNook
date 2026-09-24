//! Sending TikTok LIVE chat as the signed-in account.
//!
//! TikTok's chat send (`POST /webcast/room/chat/`) is on its page's list of
//! protected requests: the page's security SDK adds a CSRF token and a
//! bot-defence signature, and the session cookies say who is talking, so a
//! request built outside the page is refused. A message therefore goes out the
//! way tiktok.com sends one: from a TikTok page in a hidden window on the
//! sign-in profile, which enters the room once and then posts each message
//! through the page's own fetch, as typed text.
//!
//! Nothing here sends on its own. A message goes out only when someone presses
//! send, and at most one per `SEND_GAP`: TikTok's own box does not take a burst
//! either, and a burst from here would look automated.

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde_json::Value;
use std::time::{Duration, Instant};

use crate::services::providers::SendOutcome;

const SEND_GAP: Duration = Duration::from_millis(900);
/// Includes loading the page for a first message.
#[cfg(desktop)]
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the page stays open after the last message, so a conversation
/// does not reload it for every line.
#[cfg(desktop)]
const IDLE_CLOSE: Duration = Duration::from_secs(180);
/// A signed-in TikTok page with the live app loaded, and a feed request of its
/// own to take the device parameters from.
#[cfg(desktop)]
const PAGE: &str = "https://www.tiktok.com/live/following";

/// What the page reports: TikTok's answer to the message, untouched, and to
/// entering the room when this message entered it.
#[derive(Debug, Deserialize)]
struct ChatAnswer {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    enter: Option<String>,
}

/// One message at a time, and when the last one went.
static SENDING: Lazy<tokio::sync::Mutex<Option<Instant>>> = Lazy::new(|| tokio::sync::Mutex::new(None));

#[cfg(desktop)]
static CHAT_PAGE: Lazy<crate::services::providers::tiktok_feed::window::HiddenPage> = Lazy::new(|| {
    crate::services::providers::tiktok_feed::window::HiddenPage::new(
        "tiktok-chat",
        PAGE,
        crate::services::tiktok_auth_service::tiktok_profile_dir,
        IDLE_CLOSE,
        "TikTok chat",
    )
});

/// Send `text` into room `room_id` as the signed-in account.
pub async fn send(room_id: &str, text: &str) -> Result<SendOutcome> {
    let mut last = SENDING.lock().await;
    if let Some(at) = *last {
        let wait = SEND_GAP.saturating_sub(at.elapsed());
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
    let answer = ask(room_id, text).await;
    *last = Some(Instant::now());
    let answer = answer?;
    if let Some(enter) = answer.enter.as_deref() {
        let code = serde_json::from_str::<Value>(enter)
            .ok()
            .and_then(|j| j.get("status_code").and_then(Value::as_i64));
        if code != Some(0) {
            log::warn!("[TikTok] entering room {} for chat answered {:?}", room_id, code);
        }
    }
    if !answer.ok {
        return Ok(not_sent(format!(
            "TikTok chat could not be reached: {}",
            answer.error.unwrap_or_else(|| "no reason given".into())
        )));
    }
    Ok(read_chat_answer(&answer.body))
}

/// Close the page. Called on sign-out: it holds that account's session.
pub fn forget() {
    #[cfg(desktop)]
    CHAT_PAGE.close();
}

#[cfg(desktop)]
async fn ask(room_id: &str, text: &str) -> Result<ChatAnswer> {
    use crate::services::providers::tiktok_feed::window::handler_call;
    let args = format!("{}, {}", serde_json::to_string(room_id)?, serde_json::to_string(text)?);
    CHAT_PAGE
        .ask(|id| handler_call(id, "__snTikTokChat", &args), SEND_TIMEOUT)
        .await
}

// The page is a hidden webview, which is a desktop technique.
#[cfg(not(desktop))]
async fn ask(_room_id: &str, _text: &str) -> Result<ChatAnswer> {
    Err(anyhow::anyhow!("Sending to TikTok isn't available on this device yet"))
}

fn not_sent(reason: String) -> SendOutcome {
    SendOutcome {
        message_id: None,
        is_sent: false,
        drop_reason: Some(reason),
    }
}

/// TikTok's answer to a chat send. On a refusal TikTok says why in words meant
/// for the person typing (slow mode, a blocked word, muted), which is passed on
/// as it is.
fn read_chat_answer(body: &str) -> SendOutcome {
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return not_sent("TikTok gave an answer that could not be read".into());
    };
    let code = json.get("status_code").and_then(Value::as_i64).unwrap_or(-1);
    let data = json.get("data");
    if code == 0 {
        return SendOutcome {
            message_id: data
                .and_then(|d| d.get("msg_id_str"))
                .and_then(Value::as_str)
                .map(str::to_string),
            is_sent: true,
            drop_reason: None,
        };
    }
    let said = data
        .and_then(|d| d.get("prompts").or_else(|| d.get("message")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    not_sent(said.unwrap_or_else(|| match code {
        20003 => "Sign in to TikTok again to chat".into(),
        _ => format!("TikTok did not send it (code {code})"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sent_message_carries_its_id() {
        let out = read_chat_answer(&json!({ "status_code": 0, "data": { "msg_id_str": "7689" } }).to_string());
        assert!(out.is_sent);
        assert_eq!(out.message_id.as_deref(), Some("7689"));
    }

    #[test]
    fn a_refusal_says_what_tiktok_said() {
        let out = read_chat_answer(
            &json!({ "status_code": 4003082, "data": { "prompts": "You're sending messages too fast" } }).to_string(),
        );
        assert!(!out.is_sent);
        assert_eq!(out.drop_reason.as_deref(), Some("You're sending messages too fast"));
    }

    #[test]
    fn a_refusal_without_words_still_says_something() {
        let out = read_chat_answer(&json!({ "status_code": 20003, "data": {} }).to_string());
        assert_eq!(out.drop_reason.as_deref(), Some("Sign in to TikTok again to chat"));
        let out = read_chat_answer("<html>");
        assert!(!out.is_sent);
    }
}
