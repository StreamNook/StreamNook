//! Kick's official API for the automation engine: a chat send and a
//! moderation call that each classify their own answer.
//!
//! The shared Kick client has no timeout, so a stalled answer would hold an
//! executor slot and its booking forever. These doors carry their own: every
//! request is built with `.timeout(io.timeout)` (15 s in the app, injected by
//! tests so none waits that long), and a timeout is `Unknown`, because the
//! request may have reached Kick. The manual paths (`ChatProvider::send`,
//! `ban_user`, `delete_message`) are not touched.
//!
//! The answer table (PLAN § 4.5): a 2xx whose `data.is_sent` is `false` is
//! `Refused` and a 2xx without the field, or with a body that cannot be read,
//! is `Unknown`; a 5xx is `Unknown`; any other status is `Refused`; a
//! transport error is `Refused` only when the request never left
//! (`is_connect`, `is_builder`) and `Unknown` otherwise.
//!
//! Who posts is checked twice by the door itself: the connected account's id
//! must equal the one the rule sends as, before the token is read and again
//! after it, with no await between the second check and the request. A
//! refresh that moves to another account is refused before anything is sent.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

/// Kick's official API.
pub const KICK_API: &str = "https://api.kick.com";
/// How long one engine call to Kick may take in all.
pub const KICK_DOOR_TIMEOUT: Duration = Duration::from_secs(15);

/// Words when no Kick account is connected.
pub const CONNECT_KICK_WORDS: &str = "Connect your Kick account to run this rule";
/// Words when Kick is signed in as a different account than the rule's.
pub const OTHER_ACCOUNT_WORDS: &str = "Kick is signed in as another account";
/// Words when the channel's id is not known yet.
pub const UNRESOLVED_WORDS: &str = "Kick hasn't resolved this channel yet";

/// How a Kick call ended, read from Kick's answer without defaults.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KickOutcome {
    Sent {
        message_id: Option<String>,
    },
    /// Nothing posted. `status` is the HTTP status when Kick answered one
    /// (it picks the hold after a refusal).
    Refused {
        reason: String,
        status: Option<u16>,
    },
    /// It may have posted.
    Unknown,
}

/// A moderation call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KickModeration {
    /// Time `target_user_id` out in `broadcaster_user_id`'s chat.
    Timeout {
        broadcaster_user_id: u64,
        target_user_id: u64,
        minutes: u32,
        reason: Option<String>,
    },
    /// Delete one message by its Kick id.
    Delete { message_id: String },
}

/// The client, address and time limit one call uses.
#[derive(Clone)]
pub struct KickIo {
    pub client: reqwest::Client,
    pub base: String,
    pub timeout: Duration,
}

impl KickIo {
    /// The app's: Kick's address, 15 s.
    pub fn real() -> KickIo {
        KickIo {
            client: crate::services::http::client_unbounded().clone(),
            base: KICK_API.to_string(),
            timeout: KICK_DOOR_TIMEOUT,
        }
    }
}

/// What the doors read about the connected account and the channel.
#[async_trait]
pub(crate) trait KickSource: Send + Sync {
    /// The connected account's numeric id, a pure cache read.
    fn account_id(&self) -> Option<String>;
    /// The account's access token (it may refresh).
    async fn access_token(&self) -> Option<String>;
    /// The channel's broadcaster user id, once resolved.
    fn broadcaster_id(&self, slug: &str) -> Option<u64>;
    /// `text` with the channel's native emote names as Kick's tokens.
    fn inline_emotes(&self, slug: &str, text: &str) -> String;
}

/// The app's own reads.
pub(crate) struct RealKick;

#[async_trait]
impl KickSource for RealKick {
    fn account_id(&self) -> Option<String> {
        crate::services::kick_auth_service::account_id()
    }

    async fn access_token(&self) -> Option<String> {
        crate::services::kick_auth_service::access_token().await
    }

    fn broadcaster_id(&self, slug: &str) -> Option<u64> {
        super::kick::channel_meta(slug).and_then(|m| m.user_id)
    }

    fn inline_emotes(&self, slug: &str, text: &str) -> String {
        super::kick_emotes::inline_native_emotes(slug, text)
    }
}

fn refused(reason: &str) -> KickOutcome {
    KickOutcome::Refused {
        reason: reason.to_string(),
        status: None,
    }
}

fn json_message(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get("message")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The words for a refused status: Kick's own message when the body has one.
pub fn refusal_words(status: u16, body: &[u8]) -> String {
    if status == 401 {
        return "Kick rejected the sign-in (401)".to_string();
    }
    match json_message(body) {
        Some(message) => format!(
            "Kick refused it ({status}): {}",
            message.chars().take(120).collect::<String>()
        ),
        None => format!("Kick refused it ({status})"),
    }
}

/// A chat send's answer.
pub fn classify_send(status: u16, body: &[u8]) -> KickOutcome {
    if status >= 500 {
        return KickOutcome::Unknown;
    }
    if !(200..300).contains(&status) {
        return KickOutcome::Refused {
            reason: refusal_words(status, body),
            status: Some(status),
        };
    }
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return KickOutcome::Unknown;
    };
    match json.pointer("/data/is_sent").and_then(Value::as_bool) {
        Some(true) => KickOutcome::Sent {
            message_id: json
                .pointer("/data/message_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        },
        Some(false) => refused("Kick didn't accept the message"),
        None => KickOutcome::Unknown,
    }
}

/// A moderation call's answer: a 2xx is done, whatever its body.
pub fn classify_moderation(status: u16, body: &[u8]) -> KickOutcome {
    if status >= 500 {
        KickOutcome::Unknown
    } else if (200..300).contains(&status) {
        KickOutcome::Sent { message_id: None }
    } else {
        KickOutcome::Refused {
            reason: refusal_words(status, body),
            status: Some(status),
        }
    }
}

/// A transport error: refused only when nothing could have left.
pub fn classify_transport(e: &reqwest::Error) -> KickOutcome {
    if e.is_connect() || e.is_builder() {
        refused("Couldn't reach Kick")
    } else {
        KickOutcome::Unknown
    }
}

async fn finish(
    response: reqwest::Response,
    classify: fn(u16, &[u8]) -> KickOutcome,
) -> KickOutcome {
    let status = response.status().as_u16();
    match response.bytes().await {
        Ok(bytes) => classify(status, &bytes),
        // Kick answered but the body did not arrive: a refusing status still
        // reads as its refusal, a 2xx or 5xx as Unknown.
        Err(_) if status >= 500 || (200..300).contains(&status) => KickOutcome::Unknown,
        Err(_) => KickOutcome::Refused {
            reason: refusal_words(status, &[]),
            status: Some(status),
        },
    }
}

fn wrong_account(src: &dyn KickSource, expected: &str) -> Option<KickOutcome> {
    match src.account_id() {
        Some(id) if id == expected => None,
        Some(_) => Some(refused(OTHER_ACCOUNT_WORDS)),
        None => Some(refused(CONNECT_KICK_WORDS)),
    }
}

/// One chat message to Kick's official API (replies carry the parent's id).
pub(crate) async fn send_with(
    io: &KickIo,
    src: &dyn KickSource,
    expected: &str,
    slug: &str,
    text: &str,
    reply_to: Option<&str>,
) -> KickOutcome {
    if let Some(refusal) = wrong_account(src, expected) {
        return refusal;
    }
    let Some(token) = src.access_token().await else {
        return refused(CONNECT_KICK_WORDS);
    };
    // SECOND-CHECK: a refresh keeps the account, but the check is made again
    // and nothing awaits between it and the request.
    if let Some(refusal) = wrong_account(src, expected) {
        return refusal;
    }
    let slug = slug.trim().to_lowercase();
    let Some(broadcaster) = src.broadcaster_id(&slug) else {
        return refused(UNRESOLVED_WORDS);
    };
    let mut body = json!({
        "broadcaster_user_id": broadcaster,
        "content": src.inline_emotes(&slug, text),
        "type": "user",
    });
    if let Some(parent) = reply_to.map(str::trim).filter(|p| !p.is_empty()) {
        body["reply_to_message_id"] = json!(parent);
    }
    let request = io
        .client
        .post(format!("{}/public/v1/chat", io.base))
        .bearer_auth(&token)
        .timeout(io.timeout)
        .json(&body);
    match request.send().await {
        Err(e) => classify_transport(&e),
        Ok(response) => finish(response, classify_send).await,
    }
}

fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A timeout or a delete through Kick's official API.
pub(crate) async fn moderate_with(
    io: &KickIo,
    src: &dyn KickSource,
    expected: &str,
    op: &KickModeration,
) -> KickOutcome {
    if let Some(refusal) = wrong_account(src, expected) {
        return refusal;
    }
    let Some(token) = src.access_token().await else {
        return refused(CONNECT_KICK_WORDS);
    };
    // SECOND-CHECK: as in the send.
    if let Some(refusal) = wrong_account(src, expected) {
        return refusal;
    }
    let request = match op {
        KickModeration::Timeout {
            broadcaster_user_id,
            target_user_id,
            minutes,
            reason,
        } => {
            let mut body = json!({
                "broadcaster_user_id": broadcaster_user_id,
                "user_id": target_user_id,
                "duration": (*minutes).clamp(1, crate::services::moderation::KICK_MAX_TIMEOUT_MINUTES),
            });
            if let Some(reason) = reason.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
                body["reason"] = json!(reason.chars().take(100).collect::<String>());
            }
            io.client
                .post(format!("{}/public/v1/moderation/bans", io.base))
                .json(&body)
        }
        KickModeration::Delete { message_id } => {
            // The id is a path segment: one that is not a plain id is never sent.
            if !plain_id(message_id) {
                return refused("That message has no id Kick can delete");
            }
            io.client
                .delete(format!("{}/public/v1/chat/{message_id}", io.base))
        }
    };
    match request.bearer_auth(&token).timeout(io.timeout).send().await {
        Err(e) => classify_transport(&e),
        Ok(response) => finish(response, classify_moderation).await,
    }
}

/// The engine's Kick send: `expected_account` is the Kick account id the rule
/// sends as. Never returns an `anyhow` error: every end is one of the three.
pub async fn send_classified(
    expected_account: &str,
    slug: &str,
    text: &str,
    reply_to: Option<&str>,
) -> KickOutcome {
    send_with(
        &KickIo::real(),
        &RealKick,
        expected_account,
        slug,
        text,
        reply_to,
    )
    .await
}

/// The engine's Kick timeout or delete, as above.
pub async fn moderate_classified(expected_account: &str, op: &KickModeration) -> KickOutcome {
    moderate_with(&KickIo::real(), &RealKick, expected_account, op).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // ---- a loopback Kick: nothing here reaches the real service ----------------

    enum Answer {
        Raw(String),
        Silent,
        CutBody,
    }

    fn http(status: u16, body: &str) -> Answer {
        Answer::Raw(format!(
            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    struct Server {
        base: String,
        accepted: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<String>,
    }

    fn request_complete(seen: &[u8]) -> bool {
        let text = String::from_utf8_lossy(seen);
        let Some(end) = text.find("\r\n\r\n") else {
            return false;
        };
        let head = text[..end].to_ascii_lowercase();
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        seen.len() >= end + 4 + length
    }

    async fn serve(answer: Answer) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            while !request_complete(&seen) {
                let n = socket.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            match answer {
                Answer::Raw(text) => {
                    let _ = socket.write_all(text.as_bytes()).await;
                    let _ = socket.shutdown().await;
                }
                Answer::Silent => tokio::time::sleep(Duration::from_secs(30)).await,
                Answer::CutBody => {
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 500\r\n\r\n{\"data\"")
                        .await;
                    let _ = socket.shutdown().await;
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        });
        Server {
            base,
            accepted,
            task,
        }
    }

    fn body_of(request: &str) -> Value {
        let (_, body) = request.split_once("\r\n\r\n").expect("a request body");
        serde_json::from_str(body).unwrap_or(Value::Null)
    }

    fn io(base: &str, timeout: Duration) -> KickIo {
        KickIo {
            client: reqwest::Client::new(),
            base: base.to_string(),
            timeout,
        }
    }

    struct FakeSource {
        accounts: Mutex<VecDeque<Option<String>>>,
        last: Mutex<Option<String>>,
        token: Option<String>,
        broadcaster: Option<u64>,
        token_reads: AtomicUsize,
    }

    impl FakeSource {
        fn signed_in(id: &str) -> FakeSource {
            FakeSource::script(&[Some(id)])
        }

        /// The account id each read answers, in order; the last repeats.
        fn script(ids: &[Option<&str>]) -> FakeSource {
            FakeSource {
                accounts: Mutex::new(ids.iter().map(|i| i.map(str::to_string)).collect()),
                last: Mutex::new(None),
                token: Some("kick-token".into()),
                broadcaster: Some(5001),
                token_reads: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl KickSource for FakeSource {
        fn account_id(&self) -> Option<String> {
            let mut queue = self.accounts.lock().unwrap();
            let mut last = self.last.lock().unwrap();
            if let Some(next) = queue.pop_front() {
                *last = next;
            }
            last.clone()
        }
        async fn access_token(&self) -> Option<String> {
            self.token_reads.fetch_add(1, Ordering::SeqCst);
            self.token.clone()
        }
        fn broadcaster_id(&self, _slug: &str) -> Option<u64> {
            self.broadcaster
        }
        fn inline_emotes(&self, _slug: &str, text: &str) -> String {
            text.to_string()
        }
    }

    const ME: &str = "kick-me";
    const SENT: &str = r#"{"data":{"is_sent":true,"message_id":"m-1"},"message":"OK"}"#;

    async fn send(server: &Server, timeout: Duration, reply: Option<&str>) -> KickOutcome {
        send_with(
            &io(&server.base, timeout),
            &FakeSource::signed_in(ME),
            ME,
            "Alpha",
            "hello",
            reply,
        )
        .await
    }

    // ---- the send's answer table ---------------------------------------------

    #[tokio::test]
    async fn a_2xx_with_is_sent_true_is_sent_and_the_request_is_exact() {
        let server = serve(http(200, SENT)).await;
        let out = send(&server, Duration::from_secs(2), None).await;
        assert_eq!(
            out,
            KickOutcome::Sent {
                message_id: Some("m-1".into())
            }
        );
        let request = server.task.await.unwrap();
        assert!(
            request.starts_with("POST /public/v1/chat HTTP/1.1"),
            "{request}"
        );
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer kick-token"));
        let body = body_of(&request);
        assert_eq!(body["broadcaster_user_id"], 5001);
        assert_eq!(body["content"], "hello");
        assert_eq!(body["type"], "user");
        assert!(body.get("reply_to_message_id").is_none());
    }

    #[tokio::test]
    async fn a_reply_carries_the_parent_message_id() {
        let server = serve(http(200, SENT)).await;
        send(&server, Duration::from_secs(2), Some("parent-uuid")).await;
        assert_eq!(
            body_of(&server.task.await.unwrap())["reply_to_message_id"],
            "parent-uuid"
        );
    }

    #[tokio::test]
    async fn a_2xx_with_is_sent_false_is_refused_and_releases() {
        let server = serve(http(200, r#"{"data":{"is_sent":false},"message":"OK"}"#)).await;
        let out = send(&server, Duration::from_secs(2), None).await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: "Kick didn't accept the message".into(),
                status: None
            }
        );
    }

    #[tokio::test]
    async fn a_2xx_without_the_field_or_with_an_unreadable_body_is_unknown() {
        for body in [
            r#"{"data":{},"message":"OK"}"#,
            r#"{"message":"OK"}"#,
            "not json",
            "",
        ] {
            let server = serve(http(200, body)).await;
            assert_eq!(
                send(&server, Duration::from_secs(2), None).await,
                KickOutcome::Unknown,
                "{body:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_5xx_is_unknown() {
        for status in [500, 502, 503] {
            let server = serve(http(status, "{}")).await;
            assert_eq!(
                send(&server, Duration::from_secs(2), None).await,
                KickOutcome::Unknown,
                "{status}"
            );
        }
    }

    #[tokio::test]
    async fn a_4xx_is_refused_with_its_status_and_a_401_says_the_sign_in() {
        for status in [400, 403, 404, 429] {
            let server = serve(http(status, r#"{"message":"You are banned"}"#)).await;
            match send(&server, Duration::from_secs(2), None).await {
                KickOutcome::Refused {
                    reason,
                    status: got,
                } => {
                    assert_eq!(got, Some(status));
                    assert_eq!(
                        reason,
                        format!("Kick refused it ({status}): You are banned")
                    );
                }
                other => panic!("{status}: {other:?}"),
            }
        }
        let server = serve(http(401, "{}")).await;
        assert_eq!(
            send(&server, Duration::from_secs(2), None).await,
            KickOutcome::Refused {
                reason: "Kick rejected the sign-in (401)".into(),
                status: Some(401)
            }
        );
    }

    #[tokio::test]
    async fn a_silent_kick_ends_at_the_injected_timeout_as_unknown() {
        let server = serve(Answer::Silent).await;
        let started = Instant::now();
        let out = send(&server, Duration::from_millis(150), None).await;
        assert_eq!(out, KickOutcome::Unknown);
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the test never waits the real 15 s"
        );
        server.task.abort();
    }

    #[tokio::test]
    async fn a_refused_connection_and_a_bad_address_are_refused() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let out = send_with(
            &io(&closed, Duration::from_secs(2)),
            &FakeSource::signed_in(ME),
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: "Couldn't reach Kick".into(),
                status: None
            }
        );
        let out = send_with(
            &io("not a url", Duration::from_secs(2)),
            &FakeSource::signed_in(ME),
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: "Couldn't reach Kick".into(),
                status: None
            },
            "is_builder"
        );
    }

    #[tokio::test]
    async fn a_body_that_stops_after_a_2xx_is_unknown() {
        let server = serve(Answer::CutBody).await;
        assert_eq!(
            send(&server, Duration::from_secs(2), None).await,
            KickOutcome::Unknown
        );
    }

    // ---- who posts ---------------------------------------------------------------

    #[tokio::test]
    async fn another_connected_account_or_none_sends_nothing_and_reads_no_token() {
        let server = serve(Answer::Silent).await;
        let source = FakeSource::signed_in("kick-other");
        let out = send_with(
            &io(&server.base, Duration::from_secs(2)),
            &source,
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: OTHER_ACCOUNT_WORDS.into(),
                status: None
            }
        );
        assert_eq!(source.token_reads.load(Ordering::SeqCst), 0);
        let none = FakeSource::script(&[None]);
        let out = send_with(
            &io(&server.base, Duration::from_secs(2)),
            &none,
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: CONNECT_KICK_WORDS.into(),
                status: None
            }
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.accepted.load(Ordering::SeqCst), 0, "no request left");
        server.task.abort();
    }

    #[tokio::test]
    async fn a_switch_inside_the_token_read_is_refused_before_any_request() {
        let server = serve(Answer::Silent).await;
        // The first read names the rule's account, the second (after the token) another.
        let source = FakeSource::script(&[Some(ME), Some("kick-other")]);
        let out = send_with(
            &io(&server.base, Duration::from_secs(2)),
            &source,
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: OTHER_ACCOUNT_WORDS.into(),
                status: None
            }
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.accepted.load(Ordering::SeqCst), 0);
        server.task.abort();
    }

    #[tokio::test]
    async fn an_unresolved_channel_is_refused_not_guessed() {
        let server = serve(Answer::Silent).await;
        let mut source = FakeSource::signed_in(ME);
        source.broadcaster = None;
        let out = send_with(
            &io(&server.base, Duration::from_secs(2)),
            &source,
            ME,
            "alpha",
            "hi",
            None,
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: UNRESOLVED_WORDS.into(),
                status: None
            }
        );
        server.task.abort();
    }

    // ---- moderation ----------------------------------------------------------------

    fn timeout_op() -> KickModeration {
        KickModeration::Timeout {
            broadcaster_user_id: 5001,
            target_user_id: 77,
            minutes: 10,
            reason: Some("spam".into()),
        }
    }

    async fn moderate(server: &Server, op: &KickModeration, timeout: Duration) -> KickOutcome {
        moderate_with(
            &io(&server.base, timeout),
            &FakeSource::signed_in(ME),
            ME,
            op,
        )
        .await
    }

    #[tokio::test]
    async fn a_timeout_is_a_ban_with_a_duration_and_a_2xx_is_sent() {
        let server = serve(http(200, "{}")).await;
        assert_eq!(
            moderate(&server, &timeout_op(), Duration::from_secs(2)).await,
            KickOutcome::Sent { message_id: None }
        );
        let request = server.task.await.unwrap();
        assert!(
            request.starts_with("POST /public/v1/moderation/bans HTTP/1.1"),
            "{request}"
        );
        let body = body_of(&request);
        assert_eq!(
            (body["broadcaster_user_id"].clone(), body["user_id"].clone()),
            (json!(5001), json!(77))
        );
        assert_eq!(
            (body["duration"].clone(), body["reason"].clone()),
            (json!(10), json!("spam"))
        );
    }

    #[tokio::test]
    async fn a_timeout_longer_than_a_week_is_clamped_and_never_permanent() {
        let server = serve(http(200, "{}")).await;
        let op = KickModeration::Timeout {
            broadcaster_user_id: 5001,
            target_user_id: 77,
            minutes: 99_999,
            reason: None,
        };
        moderate(&server, &op, Duration::from_secs(2)).await;
        let body = body_of(&server.task.await.unwrap());
        assert_eq!(
            body["duration"],
            crate::services::moderation::KICK_MAX_TIMEOUT_MINUTES
        );
        let zero = KickModeration::Timeout {
            broadcaster_user_id: 5001,
            target_user_id: 77,
            minutes: 0,
            reason: None,
        };
        let server = serve(http(200, "{}")).await;
        moderate(&server, &zero, Duration::from_secs(2)).await;
        assert_eq!(
            body_of(&server.task.await.unwrap())["duration"],
            1,
            "zero never lifts or bans"
        );
    }

    #[tokio::test]
    async fn a_delete_is_a_delete_on_the_message_path() {
        let server = serve(http(204, "")).await;
        let op = KickModeration::Delete {
            message_id: "9b4c2f1e-0d3a".into(),
        };
        assert_eq!(
            moderate(&server, &op, Duration::from_secs(2)).await,
            KickOutcome::Sent { message_id: None }
        );
        let request = server.task.await.unwrap();
        assert!(
            request.starts_with("DELETE /public/v1/chat/9b4c2f1e-0d3a HTTP/1.1"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn a_delete_id_that_is_not_plain_is_never_sent() {
        let server = serve(Answer::Silent).await;
        for id in ["", "../bans", "a/b", "x y", &"a".repeat(65)] {
            let op = KickModeration::Delete {
                message_id: id.to_string(),
            };
            assert!(
                matches!(
                    moderate(&server, &op, Duration::from_secs(2)).await,
                    KickOutcome::Refused { .. }
                ),
                "{id:?}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.accepted.load(Ordering::SeqCst), 0);
        server.task.abort();
    }

    #[tokio::test]
    async fn moderation_answers_follow_the_table() {
        for (status, want_refused) in [
            (403, true),
            (404, true),
            (429, true),
            (500, false),
            (503, false),
        ] {
            let server = serve(http(status, "{}")).await;
            let out = moderate(&server, &timeout_op(), Duration::from_secs(2)).await;
            assert_eq!(
                matches!(out, KickOutcome::Refused { .. }),
                want_refused,
                "{status}: {out:?}"
            );
            assert_eq!(out == KickOutcome::Unknown, !want_refused, "{status}");
        }
        let server = serve(Answer::Silent).await;
        let started = Instant::now();
        assert_eq!(
            moderate(&server, &timeout_op(), Duration::from_millis(150)).await,
            KickOutcome::Unknown
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        server.task.abort();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let out = moderate_with(
            &io(&closed, Duration::from_secs(2)),
            &FakeSource::signed_in(ME),
            ME,
            &timeout_op(),
        )
        .await;
        assert!(
            matches!(out, KickOutcome::Refused { status: None, .. }),
            "a connect error is Refused"
        );
    }

    #[tokio::test]
    async fn moderation_checks_the_account_twice_too() {
        let server = serve(Answer::Silent).await;
        let source = FakeSource::script(&[Some(ME), Some("kick-other")]);
        let out = moderate_with(
            &io(&server.base, Duration::from_secs(2)),
            &source,
            ME,
            &timeout_op(),
        )
        .await;
        assert_eq!(
            out,
            KickOutcome::Refused {
                reason: OTHER_ACCOUNT_WORDS.into(),
                status: None
            }
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(server.accepted.load(Ordering::SeqCst), 0);
        server.task.abort();
    }

    // ---- pure tables and pins --------------------------------------------------------

    #[test]
    fn the_answer_tables_are_pure() {
        assert_eq!(
            classify_send(200, SENT.as_bytes()),
            KickOutcome::Sent {
                message_id: Some("m-1".into())
            }
        );
        assert_eq!(
            classify_send(200, br#"{"data":{"is_sent":true}}"#),
            KickOutcome::Sent { message_id: None }
        );
        assert_eq!(
            classify_send(204, b""),
            KickOutcome::Unknown,
            "a 2xx with no body is not a confirmation"
        );
        assert_eq!(classify_send(500, b""), KickOutcome::Unknown);
        assert_eq!(
            classify_moderation(200, b""),
            KickOutcome::Sent { message_id: None }
        );
        assert_eq!(
            classify_moderation(204, b"not json"),
            KickOutcome::Sent { message_id: None }
        );
        assert_eq!(refusal_words(403, b""), "Kick refused it (403)");
        let long = format!(r#"{{"message":"{}"}}"#, "x".repeat(300));
        assert_eq!(
            refusal_words(400, long.as_bytes()).chars().count(),
            "Kick refused it (400): ".len() + 120
        );
    }

    #[test]
    fn the_manual_send_and_its_answer_shape_are_untouched() {
        let kick = include_str!("kick.rs");
        let send = kick
            .split("async fn send(&self, channel: &str")
            .nth(1)
            .and_then(|rest| rest.split("async fn send_capability").next())
            .expect("the manual send");
        assert!(
            send.contains("is_sent: true"),
            "a 2xx is still sent without reading the flag"
        );
        assert!(
            send.contains("Kick send failed (HTTP {status})"),
            "any non-2xx is still not sent"
        );
        assert!(send.contains("Connect your Kick account to send"));
        assert!(
            send.contains("provider_record_typed"),
            "a typed send is counted in Kick's pace, never refused by it"
        );
        assert!(
            !send.contains("send_classified") && !send.contains("kick_door"),
            "the composers keep their own path"
        );
        let shape = serde_json::to_value(crate::services::providers::SendOutcome {
            message_id: None,
            is_sent: false,
            drop_reason: Some("x".into()),
        })
        .unwrap();
        assert_eq!(
            shape,
            json!({ "message_id": null, "is_sent": false, "drop_reason": "x" })
        );
    }

    #[test]
    fn every_engine_request_carries_a_timeout_and_the_manual_calls_are_not_used() {
        let src = include_str!("kick_door.rs");
        let live = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            live.matches(".timeout(io.timeout)").count(),
            2,
            "the send and the moderation call"
        );
        assert_eq!(
            live.matches("client_unbounded()").count(),
            1,
            "only KickIo::real names the shared client"
        );
        assert!(!live.contains("ban_user(") && !live.contains("delete_message("));
        // The second account check sits right before the request, with no await after it.
        let tail = live
            .split("// SECOND-CHECK")
            .nth(1)
            .and_then(|t| t.split(".send().await").next())
            .expect("the send");
        assert!(
            !tail.contains(".await"),
            "no await between the second check and the request"
        );
    }
}
