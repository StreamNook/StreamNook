//! Sending a YouTube live-chat message the way YouTube's own web client sends it.
//!
//! The web client posts `live_chat/send_message` with:
//!
//! - the `params` token and `clientIdPrefix` that the SIGNED-IN live_chat page hands
//!   it in `sendLiveChatMessageEndpoint` (the send button's service endpoint);
//! - a `clientMessageId` (that prefix plus a counter);
//! - the client version and visitorData of that same signed-in page, in the context
//!   and as the `X-Youtube-Client-*` / `X-Goog-Visitor-Id` headers.
//!
//! The older path built `params` locally and borrowed the client version and
//! visitorData from the ANONYMOUS watch page fetched at chat resolve, with no
//! `clientMessageId`. YouTube rejects requests it considers malformed with HTTP 400
//! "Request contains an invalid argument" and names no field, so this mirrors the
//! site's request instead of reasoning about which part it dislikes. Writes are where
//! YouTube is strict: the subscribe write failed the same way until it read its
//! endpoint and visitorData off the signed-in page.
//!
//! The page also says why a send cannot work at all (members-only, subscribers-only,
//! no channel on the account, chat off), which turns a raw error into a reason.

use super::youtube::{decode_json_escapes, extract_json, json_str_after, YouTubeChannelMeta};
use crate::services::youtube_auth_service;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// YouTube's live-chat input stops at 200 characters; a longer message is refused.
pub(crate) const MAX_MESSAGE_CHARS: usize = 200;

/// Only used when no page supplied a version at all.
const FALLBACK_CLIENT_VERSION: &str = "2.20240101.00.00";

/// How long a page's send context is reused before the next send reloads it. The
/// token is per video and per signed-in channel, so it rarely changes mid-stream;
/// the 400 retry below covers the case where it did.
const CONTEXT_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_CACHED: usize = 16;

pub(crate) const SIGNED_OUT_MESSAGE: &str =
    "You need to be signed in to YouTube to chat here. Reconnect YouTube and try again.";

/// Everything one send needs that is not the text itself.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SendContext {
    pub params: String,
    pub client_id_prefix: Option<String>,
    pub api_key: Option<String>,
    pub client_version: Option<String>,
    pub visitor_data: Option<String>,
    /// The page the send is made from, sent as `Referer`.
    pub referer: String,
    /// True when `params` came from a signed-in page, false when built locally.
    pub from_page: bool,
}

/// What the signed-in live_chat page says about sending.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SendPage {
    /// The page offers a send button; here is its endpoint.
    Ready(SendContext),
    /// The page offers no send button and says why (or a generic reason).
    Blocked(String),
    /// The page was served signed out: the session cookies no longer authenticate.
    SignedOut,
    /// No usable page (network, consent wall, unparseable). Fall back to built params.
    Unreadable,
}

/// A rejected or failed send. `status` is None for a transport error. `body` is
/// YouTube's response (or the transport error), kept for the log, never the toast.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SendFailure {
    pub status: Option<u16>,
    pub body: String,
}

pub(crate) fn live_chat_url(video_id: &str) -> String {
    format!("https://www.youtube.com/live_chat?is_popout=1&v={}", video_id)
}

/// Read the send endpoint (or the reason there is none) off a live_chat page.
pub(crate) fn parse_send_page(html: &str, referer: &str) -> SendPage {
    // A rotted cookie set is served a signed-out page with HTTP 200; its ytcfg says so.
    if !html.contains("\"LOGGED_IN\":true") {
        return SendPage::SignedOut;
    }
    let Some(initial) = extract_json(html, "ytInitialData") else {
        return SendPage::Unreadable;
    };
    if let Some(ep) = find_key(&initial, "sendLiveChatMessageEndpoint") {
        if let Some(params) = ep
            .get("params")
            .and_then(|p| p.as_str())
            .filter(|p| !p.is_empty())
        {
            return SendPage::Ready(SendContext {
                params: params.to_string(),
                client_id_prefix: ep
                    .get("clientIdPrefix")
                    .and_then(|p| p.as_str())
                    .filter(|p| !p.is_empty())
                    .map(String::from),
                api_key: json_str_after(html, "\"INNERTUBE_API_KEY\":\""),
                client_version: json_str_after(html, "\"INNERTUBE_CONTEXT_CLIENT_VERSION\":\"")
                    .or_else(|| json_str_after(html, "\"clientVersion\":\"")),
                visitor_data: json_str_after(html, "\"visitorData\":\"")
                    .map(|v| decode_json_escapes(&v)),
                referer: referer.to_string(),
                from_page: true,
            });
        }
    }
    // No send button. Members-only, subscribers-only, an account with no channel
    // and similar cases replace the input with a restricted-participation message;
    // a chat that is off carries a plain message renderer.
    let reason = find_key(&initial, "liveChatRestrictedParticipationRenderer")
        .and_then(|r| runs_text(r.get("message")))
        .or_else(|| find_key(&initial, "messageRenderer").and_then(|r| runs_text(r.get("text"))))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match reason {
        Some(reason) => SendPage::Blocked(reason),
        // Neither a send button nor a reason: a page shape this reader does not
        // know. The locally built params still get their try rather than every
        // send being refused.
        None => SendPage::Unreadable,
    }
}

/// The request body YouTube's web client posts to `live_chat/send_message`.
pub(crate) fn build_send_body(
    client_version: &str,
    visitor_data: Option<&str>,
    params: &str,
    client_message_id: &str,
    text: &str,
) -> Value {
    let mut client = json!({
        "clientName": "WEB",
        "clientVersion": client_version,
        "hl": "en",
        "gl": "US",
    });
    if let Some(vd) = visitor_data.filter(|v| !v.is_empty()) {
        client["visitorData"] = json!(vd);
    }
    json!({
        "context": { "client": client },
        "params": params,
        "clientMessageId": client_message_id,
        "richMessage": { "textSegments": [{ "text": text }] },
    })
}

/// Retry once, and only when the params could be stale (reused from the cache or
/// built locally) and YouTube rejected the request's arguments. Params read off a
/// page a moment ago are as fresh as they get, so retrying those would only repeat
/// the same answer.
pub(crate) fn should_retry(failure: &SendFailure, params_fresh: bool) -> bool {
    !params_fresh && failure.status == Some(400)
}

/// A short, readable reason for a failed send. The raw body goes to the log.
pub(crate) fn friendly_error(failure: &SendFailure) -> String {
    let body = failure.body.as_str();
    if failure.status == Some(401) || body.contains("UNAUTHENTICATED") {
        return SIGNED_OUT_MESSAGE.to_string();
    }
    if failure.status == Some(403) || body.contains("PERMISSION_DENIED") {
        return "YouTube won't let this account chat in this stream.".to_string();
    }
    match failure.status {
        None => "Couldn't reach YouTube. Check your connection and try again.".to_string(),
        Some(429) => "You're sending messages too fast. Wait a moment and try again.".to_string(),
        Some(s) if s >= 500 => "YouTube is having trouble right now. Try again in a moment.".to_string(),
        // The send context is dropped on a 400, so the next try reads a fresh one.
        Some(400) => "YouTube didn't accept the message. Try sending it again.".to_string(),
        _ => "YouTube didn't accept the message.".to_string(),
    }
}

/// Refuse what YouTube would refuse anyway, before a round trip.
pub(crate) fn precheck_text(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return Some("Type a message first".to_string());
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Some(format!(
            "YouTube messages can be up to {} characters",
            MAX_MESSAGE_CHARS
        ));
    }
    None
}

/// What a 2xx `send_message` response means.
#[derive(Debug, PartialEq)]
pub(crate) enum SendResponse<'a> {
    /// YouTube posted the message. `item` is the rendered chat item (for the
    /// immediate echo), `cooldown_secs` the wait before the next send is allowed.
    Accepted {
        item: Option<&'a Value>,
        message_id: Option<String>,
        cooldown_secs: u64,
    },
    /// YouTube took the request but its answer carries no posted message and no
    /// refusal, only (at most) the ordinary one-second pacing. The message has
    /// most likely posted; whether it did shows when its row reaches chat.
    Unconfirmed { cooldown_secs: u64 },
    /// YouTube answered 200 but did not post the message.
    Refused(String),
}

/// Read a 2xx `send_message` response.
///
/// `timeoutDurationUsec` is NOT a timeout on its own: a posted message carries it
/// as the per-viewer cooldown (one second in an ordinary chat, the slow-mode
/// interval in a slow chat). Reading it as "you are timed out" reported a message
/// that had just posted as failed and put it back in the composer. The posted
/// message itself (`addChatItemAction`) is what says it went through.
pub(crate) fn classify_send_response(v: &Value) -> SendResponse<'_> {
    let cooldown_secs = v
        .get("timeoutDurationUsec")
        .and_then(|x| x.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| x.as_u64()))
        .map(|us| us.div_ceil(1_000_000))
        .unwrap_or(0);
    let actions = v.get("actions").and_then(|a| a.as_array());
    let item = actions.and_then(|acts| {
        acts.iter()
            .filter_map(|a| a.pointer("/addChatItemAction/item"))
            .find(|i| i.get("liveChatTextMessageRenderer").is_some())
    });
    if let Some(item) = item {
        return SendResponse::Accepted {
            item: Some(item),
            message_id: item
                .pointer("/liveChatTextMessageRenderer/id")
                .and_then(|x| x.as_str())
                .map(String::from),
            cooldown_secs,
        };
    }
    // No posted message. A refusal arrives as a system line built from a template
    // (timed out, slow mode, blocked word), so its text is the reason.
    let notice = actions.and_then(|acts| {
        acts.iter().find_map(|a| {
            let r = a.get("addLiveChatTextMessageFromTemplateAction")?;
            runs_text(
                find_key(r, "message").or_else(|| find_key(r, "text")),
            )
        })
    });
    if let Some(reason) = notice.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        return SendResponse::Refused(reason);
    }
    // A cooldown longer than the ordinary pacing, with nothing posted, is
    // YouTube holding the viewer back.
    if cooldown_secs > 1 {
        return SendResponse::Refused(format!(
            "YouTube asked you to wait {}s before sending again",
            cooldown_secs
        ));
    }
    // Nothing posted and nothing refused. Reading this as a failure is what put
    // posted messages back in the composer, so it counts as sent and the chat
    // rows confirm it (see `expect_echo`).
    SendResponse::Unconfirmed { cooldown_secs }
}

// --- Confirming a send that came back without its posted row -----------------

/// How long a send without its posted row waits for that row to reach chat
/// before the viewer is told it could not be confirmed. The chat poll answers
/// every few seconds, so this covers two or three polls.
pub(crate) const ECHO_WAIT: Duration = Duration::from_secs(12);

/// Sent to the chat windows when that wait ran out: `{ provider, channel, text }`,
/// `channel` being the chat key the message was sent to.
pub(crate) const SEND_UNCONFIRMED_EVENT: &str = "provider-send-unconfirmed";

struct PendingEcho {
    id: u64,
    channel_key: String,
    text: String,
    own_id: Option<String>,
    at: Instant,
    seen: bool,
}

static PENDING_ECHOES: OnceLock<Mutex<Vec<PendingEcho>>> = OnceLock::new();
static PENDING_COUNT: AtomicU64 = AtomicU64::new(0);
static ECHO_SEQ: AtomicU64 = AtomicU64::new(0);

fn pending_echoes() -> std::sync::MutexGuard<'static, Vec<PendingEcho>> {
    PENDING_ECHOES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Whitespace-collapsed text, for comparing what was sent with what came back.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Start watching `channel_key`'s chat rows for a message just sent as `text`
/// by the channel `own_id` (when known). Returns the id `echo_seen` takes.
pub(crate) fn expect_echo(channel_key: &str, text: &str, own_id: Option<String>) -> u64 {
    let id = ECHO_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut list = pending_echoes();
    // Anything long past its wait was never collected; do not let it build up.
    list.retain(|p| p.at.elapsed() < ECHO_WAIT * 5);
    list.push(PendingEcho {
        id,
        channel_key: channel_key.to_lowercase(),
        text: normalize(text),
        own_id: own_id.filter(|s| !s.is_empty()),
        at: Instant::now(),
        seen: false,
    });
    PENDING_COUNT.store(list.len() as u64, Ordering::Relaxed);
    id
}

/// A chat row arrived in `channel_key` from `author_id` saying `text`. It
/// confirms the oldest waiting send it matches: same text from us (or from
/// anyone when our channel id is unknown), else any row from our own channel,
/// since YouTube can turn typed emoji codes into emoji on the way back.
pub(crate) fn note_chat_row(channel_key: &str, author_id: &str, text: &str) {
    if PENDING_COUNT.load(Ordering::Relaxed) == 0 {
        return;
    }
    let key = channel_key.to_lowercase();
    let text = normalize(text);
    let mut list = pending_echoes();
    let mine = |p: &PendingEcho| p.own_id.as_deref().map_or(true, |own| own == author_id);
    let by_text = list
        .iter()
        .position(|p| !p.seen && p.channel_key == key && p.text == text && mine(p));
    let by_author = || {
        list.iter()
            .position(|p| !p.seen && p.channel_key == key && p.own_id.as_deref() == Some(author_id))
    };
    if let Some(i) = by_text.or_else(by_author) {
        list[i].seen = true;
    }
}

/// Stop watching send `id`; true when its row reached chat.
pub(crate) fn echo_seen(id: u64) -> bool {
    let mut list = pending_echoes();
    let seen = list.iter().find(|p| p.id == id).is_some_and(|p| p.seen);
    list.retain(|p| p.id != id);
    PENDING_COUNT.store(list.len() as u64, Ordering::Relaxed);
    seen
}

static SEND_SEQ: AtomicU64 = AtomicU64::new(0);

/// The web client's id: the page's `clientIdPrefix` plus a running count.
fn next_client_message_id(prefix: Option<&str>) -> String {
    match prefix.filter(|p| !p.is_empty()) {
        Some(p) => format!("{}{}", p, SEND_SEQ.fetch_add(1, Ordering::Relaxed)),
        None => uuid::Uuid::new_v4().to_string(),
    }
}

// --- Send-context cache (per video, per signed-in identity) ------------------

struct Cached {
    identity: String,
    ctx: SendContext,
    at: Instant,
}

static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_context(video_id: &str, identity: &str) -> Option<SendContext> {
    let map = cache().lock().ok()?;
    let c = map.get(video_id)?;
    (c.identity == identity && c.at.elapsed() < CONTEXT_TTL).then(|| c.ctx.clone())
}

fn remember_context(video_id: &str, identity: &str, ctx: &SendContext) {
    if let Ok(mut map) = cache().lock() {
        map.retain(|_, c| c.at.elapsed() < CONTEXT_TTL);
        if map.len() >= MAX_CACHED {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, c)| c.at)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(
            video_id.to_string(),
            Cached {
                identity: identity.to_string(),
                ctx: ctx.clone(),
                at: Instant::now(),
            },
        );
    }
}

fn forget_context(video_id: &str) {
    if let Ok(mut map) = cache().lock() {
        map.remove(video_id);
    }
}

// --- Network ---------------------------------------------------------------

/// Fetch the live_chat page as the signed-in user and read its send endpoint.
async fn load_send_page(video_id: &str) -> SendPage {
    let Some(headers) = youtube_auth_service::auth_headers() else {
        return SendPage::SignedOut;
    };
    let url = live_chat_url(video_id);
    let mut req = youtube_auth_service::page_client()
        .get(&url)
        .header("User-Agent", USER_AGENT);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[YouTube] send: chat page fetch failed: {}", e);
            return SendPage::Unreadable;
        }
    };
    if resp.url().as_str().contains("consent.youtube.com") {
        log::warn!("[YouTube] send: chat page hit the consent wall; using built params");
        return SendPage::Unreadable;
    }
    let html = match resp.text().await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("[YouTube] send: chat page read failed: {}", e);
            return SendPage::Unreadable;
        }
    };
    // The page is asked for as the session's account and channel. If YouTube
    // rendered it for another one, a restriction it reports may not be ours.
    if let Some(served_as) = youtube_auth_service::page_identity_key(&html) {
        let acting_as = youtube_auth_service::identity_key();
        if served_as != acting_as {
            log::warn!(
                "[YouTube] send: chat page for {} was served as identity {} while the session acts as {}",
                video_id,
                served_as,
                acting_as
            );
        }
    }
    let page = parse_send_page(&html, &url);
    match &page {
        SendPage::Ready(c) => log::info!(
            "[YouTube] send: chat page for {} offers a send endpoint (clientIdPrefix={}, clientVersion={})",
            video_id,
            c.client_id_prefix.is_some(),
            c.client_version.as_deref().unwrap_or("none")
        ),
        SendPage::Blocked(r) => log::info!("[YouTube] send: chat page for {} offers no input: {}", video_id, r),
        SendPage::SignedOut => log::warn!("[YouTube] send: chat page for {} was served signed out", video_id),
        SendPage::Unreadable => log::warn!(
            "[YouTube] send: chat page for {} offers neither a send endpoint nor a reason ({} bytes); using built params",
            video_id,
            html.len()
        ),
    }
    page
}

/// `load_send_page`, re-harvesting the session once if the page came back signed out.
async fn load_send_page_recovering(video_id: &str) -> SendPage {
    let page = load_send_page(video_id).await;
    if page == SendPage::SignedOut && youtube_auth_service::recover_stale_session().await {
        return load_send_page(video_id).await;
    }
    page
}

/// The pre-page path's context: locally built params and the resolve's scraped
/// values. Used only when the signed-in page cannot be read.
fn built_context(meta: &YouTubeChannelMeta, channel_id: &str, video_id: &str) -> SendContext {
    SendContext {
        params: super::youtube::send_message_params(channel_id, video_id),
        client_id_prefix: None,
        api_key: meta.api_key.clone(),
        client_version: meta.client_version.clone(),
        visitor_data: meta.visitor_data.clone(),
        referer: live_chat_url(video_id),
        from_page: false,
    }
}

async fn post_send(ctx: &SendContext, text: &str) -> Result<Value, SendFailure> {
    let Some(headers) = youtube_auth_service::auth_headers() else {
        return Err(SendFailure {
            status: Some(401),
            body: "no YouTube session".to_string(),
        });
    };
    let client_version = ctx
        .client_version
        .as_deref()
        .unwrap_or(FALLBACK_CLIENT_VERSION);
    let body = build_send_body(
        client_version,
        ctx.visitor_data.as_deref(),
        &ctx.params,
        &next_client_message_id(ctx.client_id_prefix.as_deref()),
        text,
    );
    let url = match ctx.api_key.as_deref().filter(|k| !k.is_empty()) {
        Some(key) => format!(
            "https://www.youtube.com/youtubei/v1/live_chat/send_message?key={}&prettyPrint=false",
            key
        ),
        None => "https://www.youtube.com/youtubei/v1/live_chat/send_message?prettyPrint=false".to_string(),
    };
    let mut req = crate::services::http::client_unbounded()
        .post(url)
        .header("User-Agent", USER_AGENT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", client_version)
        .header("X-Youtube-Bootstrap-Logged-In", "true")
        .header(reqwest::header::REFERER, ctx.referer.as_str());
    if let Some(vd) = ctx.visitor_data.as_deref().filter(|v| !v.is_empty()) {
        req = req.header("X-Goog-Visitor-Id", vd);
    }
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.json(&body).send().await.map_err(|e| SendFailure {
        status: None,
        body: e.to_string(),
    })?;
    let status = resp.status().as_u16();
    let raw = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(SendFailure { status: Some(status), body: raw });
    }
    let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if let Some(err) = v.get("error") {
        let code = err
            .get("code")
            .and_then(|c| c.as_u64())
            .and_then(|c| u16::try_from(c).ok())
            .unwrap_or(status);
        return Err(SendFailure { status: Some(code), body: raw });
    }
    Ok(v)
}

/// Log YouTube's full answer. The body is YouTube's error JSON: it carries no
/// cookies or tokens, and it is the part a bug report cannot show.
fn log_failure(failure: &SendFailure, ctx: &SendContext, attempt: u8) {
    let body: String = failure.body.chars().take(4000).collect();
    log::warn!(
        "[YouTube] send failed (attempt {}, params from {}, clientVersion={}, visitorData={}): HTTP {}: {}",
        attempt,
        if ctx.from_page { "the signed-in chat page" } else { "a local build" },
        ctx.client_version.as_deref().unwrap_or("none"),
        if ctx.visitor_data.is_some() { "present" } else { "absent" },
        failure
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "transport error".to_string()),
        body
    );
}

/// Send `text` to the chat of `video_id`. Ok carries YouTube's response; Err a
/// short reason for the viewer.
pub(crate) async fn send_text(
    meta: &YouTubeChannelMeta,
    channel_id: &str,
    video_id: &str,
    text: &str,
) -> Result<Value, String> {
    if let Some(reason) = precheck_text(text) {
        return Err(reason);
    }
    let identity = youtube_auth_service::identity_key();
    let (ctx, fresh) = match cached_context(video_id, &identity) {
        Some(c) => (c, false),
        None => match load_send_page_recovering(video_id).await {
            SendPage::Ready(c) => {
                remember_context(video_id, &identity, &c);
                (c, true)
            }
            SendPage::Blocked(reason) => return Err(reason),
            SendPage::SignedOut => return Err(SIGNED_OUT_MESSAGE.to_string()),
            SendPage::Unreadable => (built_context(meta, channel_id, video_id), false),
        },
    };

    let first = match post_send(&ctx, text).await {
        Ok(v) => return Ok(v),
        Err(f) => f,
    };
    log_failure(&first, &ctx, 1);
    if !should_retry(&first, fresh) {
        // A rejected request leaves nothing worth reusing: the viewer's next
        // try reads the chat page afresh.
        if first.status == Some(400) {
            forget_context(video_id);
        }
        return Err(friendly_error(&first));
    }

    log::info!("[YouTube] send: reloading the chat page for fresh send params and retrying once");
    forget_context(video_id);
    match load_send_page(video_id).await {
        SendPage::Ready(c) => {
            remember_context(video_id, &identity, &c);
            match post_send(&c, text).await {
                Ok(v) => Ok(v),
                Err(second) => {
                    log_failure(&second, &c, 2);
                    if second.status == Some(400) {
                        forget_context(video_id);
                    }
                    Err(friendly_error(&second))
                }
            }
        }
        SendPage::Blocked(reason) => Err(reason),
        SendPage::SignedOut => Err(SIGNED_OUT_MESSAGE.to_string()),
        SendPage::Unreadable => Err(friendly_error(&first)),
    }
}

// --- Small JSON helpers ----------------------------------------------------

/// The first value under `key` anywhere in a tree (depth-first).
fn find_key<'a>(root: &'a Value, key: &str) -> Option<&'a Value> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(v) = map.get(key) {
                    return Some(v);
                }
                stack.extend(map.values().rev());
            }
            Value::Array(items) => stack.extend(items.iter().rev()),
            _ => {}
        }
    }
    None
}

/// Flatten a `{ runs: [...] }` or `{ simpleText }` text object.
fn runs_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.get("simpleText").and_then(|s| s.as_str()) {
        return Some(s.to_string());
    }
    let s: String = v
        .get("runs")?
        .as_array()?
        .iter()
        .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
        .collect();
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFERER: &str = "https://www.youtube.com/live_chat?is_popout=1&v=AAAAAAAAAAA";

    /// A signed-in live_chat page, cut down to the parts the parser reads. The
    /// ytcfg and the chat tree follow a real page served 2026-10 (anonymised ids
    /// and tokens); the send button sits where the web client reads it,
    /// `liveChatMessageInputRenderer.sendButton.buttonRenderer.serviceEndpoint`.
    const SIGNED_IN_PAGE: &str = r#"<html><script>ytcfg.set({"INNERTUBE_API_KEY":"AIzaTestKey000","INNERTUBE_CONTEXT":{"client":{"hl":"en","gl":"US","visitorData":"CgtWSVNJVE9SMDAwMCi%3D","clientName":"WEB","clientVersion":"2.20261009.01.00"}},"INNERTUBE_CONTEXT_CLIENT_VERSION":"2.20261009.01.00","LOGGED_IN":true,"SESSION_INDEX":"0"});</script>
<script>window["ytInitialData"] = {"responseContext":{"visitorData":"CgtWSVNJVE9SMDAwMCi%3D"},"contents":{"liveChatRenderer":{"continuations":[{"invalidationContinuationData":{"continuation":"0ofMyANCONT"}}],"actionPanel":{"liveChatMessageInputRenderer":{"inputField":{"liveChatTextInputFieldRenderer":{"placeholder":{"runs":[{"text":"Chat..."}]},"maxCharacterLength":200}},"sendButton":{"buttonRenderer":{"icon":{"iconType":"SEND"},"serviceEndpoint":{"commandMetadata":{"webCommandMetadata":{"sendPost":true,"apiUrl":"/youtubei/v1/live_chat/send_message"}},"sendLiveChatMessageEndpoint":{"params":"Q2lrcUp3b1lWVU5CUVVGQlFVRkJRVUZCUVVGQlFVRkJRVUZCUVVGQkVndEJRVUZCUVVGQlFVRkJRUkFCR0FRJTNE","clientIdPrefix":"ABCDEFGH"}}}}}}}}};</script></html>"#;

    #[test]
    fn reads_the_send_endpoint_off_a_signed_in_page() {
        let SendPage::Ready(ctx) = parse_send_page(SIGNED_IN_PAGE, REFERER) else {
            panic!("expected a send endpoint");
        };
        assert_eq!(
            ctx.params,
            "Q2lrcUp3b1lWVU5CUVVGQlFVRkJRVUZCUVVGQlFVRkJRVUZCUVVGQkVndEJRVUZCUVVGQlFVRkJRUkFCR0FRJTNE"
        );
        assert_eq!(ctx.client_id_prefix.as_deref(), Some("ABCDEFGH"));
        assert_eq!(ctx.api_key.as_deref(), Some("AIzaTestKey000"));
        assert_eq!(ctx.client_version.as_deref(), Some("2.20261009.01.00"));
        assert_eq!(ctx.visitor_data.as_deref(), Some("CgtWSVNJVE9SMDAwMCi%3D"));
        assert_eq!(ctx.referer, REFERER);
        assert!(ctx.from_page);
    }

    #[test]
    fn a_signed_out_page_is_not_read_as_a_reason() {
        // The anonymous page has no send button either; it must say "signed out",
        // not "this chat is restricted".
        let page = SIGNED_IN_PAGE.replace("\"LOGGED_IN\":true", "\"LOGGED_IN\":false");
        assert_eq!(parse_send_page(&page, REFERER), SendPage::SignedOut);
    }

    #[test]
    fn a_restricted_chat_reports_youtube_s_own_reason() {
        let page = r#"<script>ytcfg.set({"LOGGED_IN":true});</script><script>window["ytInitialData"] = {"contents":{"liveChatRenderer":{"actionPanel":{"liveChatRestrictedParticipationRenderer":{"message":{"runs":[{"text":"Subscribers-only mode. "},{"text":"Subscribe to chat."}]}}}}}};</script>"#;
        assert_eq!(
            parse_send_page(page, REFERER),
            SendPage::Blocked("Subscribers-only mode. Subscribe to chat.".to_string())
        );
    }

    #[test]
    fn a_page_without_initial_data_falls_back() {
        let page = r#"<script>ytcfg.set({"LOGGED_IN":true});</script>"#;
        assert_eq!(parse_send_page(page, REFERER), SendPage::Unreadable);
    }

    #[test]
    fn a_page_with_neither_a_send_button_nor_a_reason_falls_back() {
        // A chat tree this reader does not know (the send button renamed or
        // wrapped): the built params get their try instead of a hard refusal.
        let page = r#"<script>ytcfg.set({"LOGGED_IN":true});</script><script>window["ytInitialData"] = {"contents":{"liveChatRenderer":{"actionPanel":{"liveChatMessageInputRenderer":{"sendButton":{"buttonRenderer":{"serviceEndpoint":{"sendLiveChatMessageCommand":{"params":"X"}}}}}}}}};</script>"#;
        assert_eq!(parse_send_page(page, REFERER), SendPage::Unreadable);
    }

    #[test]
    fn the_page_identity_compares_with_the_session_identity() {
        use crate::services::youtube_auth_service::page_identity_key;
        assert_eq!(page_identity_key(SIGNED_IN_PAGE).as_deref(), Some("0|"));
        let brand = SIGNED_IN_PAGE.replace(
            "\"SESSION_INDEX\":\"0\"",
            "\"SESSION_INDEX\":\"1\",\"DELEGATED_SESSION_ID\":\"117440512999\"",
        );
        assert_eq!(page_identity_key(&brand).as_deref(), Some("1|117440512999"));
        // A signed-out page, or one that does not name the account, says nothing.
        assert_eq!(page_identity_key(&SIGNED_IN_PAGE.replace("\"LOGGED_IN\":true", "\"LOGGED_IN\":false")), None);
        assert_eq!(page_identity_key(r#"{"LOGGED_IN":true}"#), None);
    }

    #[test]
    fn the_send_body_matches_the_web_client() {
        let body = build_send_body(
            "2.20261009.01.00",
            Some("CgtWSVNJVE9SMDAwMCi%3D"),
            "PARAMS",
            "ABCDEFGH0",
            "hello chat",
        );
        assert_eq!(
            body,
            json!({
                "context": { "client": {
                    "clientName": "WEB",
                    "clientVersion": "2.20261009.01.00",
                    "hl": "en",
                    "gl": "US",
                    "visitorData": "CgtWSVNJVE9SMDAwMCi%3D",
                }},
                "params": "PARAMS",
                "clientMessageId": "ABCDEFGH0",
                "richMessage": { "textSegments": [{ "text": "hello chat" }] },
            })
        );
        let bare = build_send_body("2.x", None, "P", "id", "t");
        assert!(bare.pointer("/context/client/visitorData").is_none());
    }

    #[test]
    fn client_message_ids_use_the_page_prefix_and_never_repeat() {
        let a = next_client_message_id(Some("ABCDEFGH"));
        let b = next_client_message_id(Some("ABCDEFGH"));
        assert!(a.starts_with("ABCDEFGH") && b.starts_with("ABCDEFGH"));
        assert_ne!(a, b);
        assert!(!next_client_message_id(None).is_empty());
    }

    #[test]
    fn only_a_400_on_possibly_stale_params_is_retried() {
        let bad = SendFailure { status: Some(400), body: String::new() };
        assert!(should_retry(&bad, false), "cached or built params get one fresh try");
        assert!(!should_retry(&bad, true), "params read a moment ago would fail the same way");
        for status in [Some(401), Some(403), Some(429), Some(500), None] {
            let f = SendFailure { status, body: String::new() };
            assert!(!should_retry(&f, false), "{:?} is not a params problem", status);
        }
    }

    #[test]
    fn error_bodies_map_to_short_reasons() {
        // The body from the report, as YouTube sends it.
        let invalid = SendFailure {
            status: Some(400),
            body: r#"{"error":{"code":400,"message":"Request contains an invalid argument.","errors":[{"message":"Request contains an invalid argument.","domain":"global","reason":"badRequest"}],"status":"INVALID_ARGUMENT"}}"#.to_string(),
        };
        let msg = friendly_error(&invalid);
        assert_eq!(msg, "YouTube didn't accept the message. Try sending it again.");
        assert!(!msg.contains('{'), "the raw body stays in the log");

        let unauth = SendFailure {
            status: Some(401),
            body: r#"{"error":{"code":401,"status":"UNAUTHENTICATED"}}"#.to_string(),
        };
        assert_eq!(friendly_error(&unauth), SIGNED_OUT_MESSAGE);

        let denied = SendFailure {
            status: Some(403),
            body: r#"{"error":{"code":403,"status":"PERMISSION_DENIED"}}"#.to_string(),
        };
        assert!(friendly_error(&denied).contains("won't let this account"));

        assert!(friendly_error(&SendFailure { status: Some(429), body: String::new() }).contains("too fast"));
        assert!(friendly_error(&SendFailure { status: Some(503), body: String::new() }).contains("trouble"));
        assert!(friendly_error(&SendFailure { status: None, body: "dns".into() }).contains("Couldn't reach"));
    }

    #[test]
    fn empty_and_overlong_messages_are_refused_locally() {
        assert!(precheck_text("   ").is_some());
        assert!(precheck_text(&"a".repeat(MAX_MESSAGE_CHARS)).is_none());
        assert!(precheck_text(&"a".repeat(MAX_MESSAGE_CHARS + 1)).is_some());
        // Characters, not bytes: 200 emoji are allowed.
        assert!(precheck_text(&"\u{1F600}".repeat(MAX_MESSAGE_CHARS)).is_none());
    }

    /// A posted message in a chat with a one-second per-viewer cooldown, shaped
    /// like the response YouTube returned to the live test (ids anonymised).
    const ACCEPTED_WITH_COOLDOWN: &str = r#"{"responseContext":{"visitorData":"CgtWSVNJVE9S"},"actions":[{"clickTrackingParams":"CAEQl98BIhMI","addChatItemAction":{"item":{"liveChatTextMessageRenderer":{"message":{"runs":[{"text":"test"}]},"authorName":{"simpleText":"@viewer"},"authorPhoto":{"thumbnails":[{"url":"https://yt4.ggpht.com/a","width":32,"height":32}]},"id":"ChwKGkNMX2twNEtac3BjREZZQjdUQWdkcjhvUzFn","timestampUsec":"1760190000000000","authorExternalChannelId":"UCxxxxxxxxxxxxxxxxxxxxxx"}},"clientId":"ABCDEFGH0"}}],"timeoutDurationUsec":"1000000"}"#;

    #[test]
    fn a_posted_message_with_a_cooldown_is_accepted() {
        let v: Value = serde_json::from_str(ACCEPTED_WITH_COOLDOWN).unwrap();
        match classify_send_response(&v) {
            SendResponse::Accepted { item, message_id, cooldown_secs } => {
                assert!(item.is_some(), "the echo needs the rendered item");
                assert_eq!(message_id.as_deref(), Some("ChwKGkNMX2twNEtac3BjREZZQjdUQWdkcjhvUzFn"));
                assert_eq!(cooldown_secs, 1);
            }
            other => panic!("a posted message was read as {:?}", other),
        }
    }

    #[test]
    fn a_slow_mode_interval_rounds_up_and_stays_a_success() {
        let raw = ACCEPTED_WITH_COOLDOWN.replace("\"1000000\"", "\"29500000\"");
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert!(matches!(
            classify_send_response(&v),
            SendResponse::Accepted { cooldown_secs: 30, .. }
        ));
    }

    #[test]
    fn a_template_notice_without_a_posted_message_is_a_refusal() {
        let v = json!({
            "actions": [{ "addLiveChatTextMessageFromTemplateAction": { "template": {
                "liveChatTextMessageRenderer": {
                    "message": { "runs": [{ "text": "Slow mode is on. Wait 25 seconds." }] },
                    "id": "local"
                }
            }}}],
            "timeoutDurationUsec": "25000000"
        });
        assert_eq!(
            classify_send_response(&v),
            SendResponse::Refused("Slow mode is on. Wait 25 seconds.".to_string())
        );
    }

    #[test]
    fn a_long_bare_cooldown_without_a_posted_message_is_a_refusal() {
        let v = json!({ "timeoutDurationUsec": "300000000" });
        assert!(matches!(classify_send_response(&v), SendResponse::Refused(_)));
    }

    #[test]
    fn the_ordinary_pacing_alone_is_unconfirmed_not_a_timeout() {
        // The answer that once read as "You're in timeout for 1s" when it came
        // without its posted row: no item, no notice, the one-second pacing.
        let v = json!({ "responseContext": { "visitorData": "CgtWSVNJVE9S" }, "timeoutDurationUsec": "1000000" });
        assert_eq!(classify_send_response(&v), SendResponse::Unconfirmed { cooldown_secs: 1 });
        // An empty 2xx too.
        assert_eq!(classify_send_response(&json!({})), SendResponse::Unconfirmed { cooldown_secs: 0 });
    }

    #[test]
    fn a_chat_row_confirms_an_unconfirmed_send() {
        let key = "youtube:echotest";
        // From our own channel, matched by text.
        let a = expect_echo(key, "hello  chat", Some("UCme".into()));
        note_chat_row("youtube:other", "UCme", "hello chat");
        note_chat_row(key, "UCsomeoneelse", "hello chat");
        assert!(!echo_seen(a), "another channel or another author does not confirm it");
        let b = expect_echo(key, "hello chat", Some("UCme".into()));
        note_chat_row("YOUTUBE:ECHOTEST", "UCme", "hello chat");
        assert!(echo_seen(b));
        // YouTube turned a typed code into an emoji: our own row still confirms.
        let c = expect_echo(key, "gg :thumbs_up:", Some("UCme".into()));
        note_chat_row(key, "UCme", "gg \u{1F44D}");
        assert!(echo_seen(c));
        // Our channel id unknown: the same text from anyone confirms it.
        let d = expect_echo(key, "rv test", None);
        note_chat_row(key, "UCwhoever", "rv test");
        assert!(echo_seen(d));
        // A row confirms one send, the oldest that matches.
        let e1 = expect_echo(key, "again", Some("UCme".into()));
        let e2 = expect_echo(key, "again", Some("UCme".into()));
        note_chat_row(key, "UCme", "again");
        assert!(echo_seen(e1));
        assert!(!echo_seen(e2));
    }

    #[test]
    fn the_cache_is_per_identity() {
        let ctx = match parse_send_page(SIGNED_IN_PAGE, REFERER) {
            SendPage::Ready(c) => c,
            other => panic!("{:?}", other),
        };
        remember_context("cachetestvid", "0|", &ctx);
        assert_eq!(cached_context("cachetestvid", "0|"), Some(ctx));
        assert_eq!(cached_context("cachetestvid", "0|1174405"), None, "another channel's token is not reused");
        forget_context("cachetestvid");
        assert_eq!(cached_context("cachetestvid", "0|"), None);
    }
}
