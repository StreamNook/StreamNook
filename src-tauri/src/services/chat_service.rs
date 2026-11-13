// This module now uses IRC for chat instead of EventSub
// Simply re-exports the IrcService as ChatService for backwards compatibility

use crate::models::settings::AppState;
use crate::services::account_store::AccountStore;
use crate::services::chat_send_limiter::{self, Ticket, ENGINE_SEND_TIMEOUT};
use crate::services::irc_service::IrcService;
use crate::services::twitch_service::{HelixSendRefused, TwitchService};
use anyhow::Result;
use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

/// Where an automation's chat message goes.
pub(crate) const HELIX_CHAT_URL: &str = "https://api.twitch.tv/helix/chat/messages";
/// The automation HTTP client's connect timeout: a stall before the request
/// leaves reports `is_connect`, so it reads as "nothing left the device".
pub const ENGINE_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// The automation HTTP client: a connect timeout and a total timeout, no
/// redirects. Production passes (3 s, 5 s); tests pass shorter ones.
pub(crate) fn engine_client(connect: Duration, total: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(total)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build the automation HTTP client")
}

static ENGINE_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| engine_client(ENGINE_CONNECT_TIMEOUT, ENGINE_SEND_TIMEOUT));

/// How an automation's send went, read from Twitch's answer: `Refused` only
/// when nothing posted, `Unknown` whenever it may have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Sent { message_id: Option<String> },
    Refused(String),
    Unknown,
}

/// The words a Helix 401 refusal starts with (Twitch refused the token
/// itself). The engine tells it from every other refusal with
/// [`is_unauthorized`].
pub const UNAUTHORIZED: &str = "Twitch rejected the sign-in";

/// Whether a `SendOutcome::Refused` reason is a Helix 401.
pub fn is_unauthorized(reason: &str) -> bool {
    reason.starts_with(UNAUTHORIZED)
}

/// A failed `send_message`. Its text is the cause's, as callers always saw
/// it; `may_have_left` says whether the message may have reached Twitch.
#[derive(Debug)]
pub struct SendError {
    pub source: anyhow::Error,
    pub may_have_left: bool,
}

impl SendError {
    fn before(source: anyhow::Error) -> SendError {
        SendError { source, may_have_left: false }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for SendError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.source()
    }
}

/// How far the Helix half of a send got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HelixStage {
    /// No Helix attempt (a `/`-line goes straight to IRC).
    NotTried,
    /// The token could not be had (a refresh timeout included).
    NoToken,
    /// The request never left: a connect or request-builder error.
    BeforeLeft,
    /// Twitch answered with this status, 400 to 499: it posted nothing. A 3xx
    /// is never one (the engine client follows no redirect, so it may have
    /// posted): it reads `AfterLeft`.
    Refused(u16),
    /// The request may have reached Twitch.
    AfterLeft,
}

/// How far the IRC half of a send got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IrcStage {
    /// It failed before writing the line (no connection, a failed re-JOIN).
    BeforeSendLine,
    /// Writing the line failed; part of it may have gone out.
    SendLineReached,
}

/// The Helix stage a failed Helix send reached.
pub(crate) fn helix_stage(e: &anyhow::Error) -> HelixStage {
    for cause in e.chain() {
        if let Some(refused) = cause.downcast_ref::<HelixSendRefused>() {
            return if (400..500).contains(&refused.status) { HelixStage::Refused(refused.status) } else { HelixStage::AfterLeft };
        }
        if let Some(sent) = cause.downcast_ref::<reqwest::Error>() {
            return if sent.is_connect() || sent.is_builder() { HelixStage::BeforeLeft } else { HelixStage::AfterLeft };
        }
    }
    HelixStage::AfterLeft
}

/// The IRC stage a failed IRC send reached: an I/O error anywhere in its
/// chain came from writing the line.
pub(crate) fn irc_stage(e: &anyhow::Error) -> IrcStage {
    if e.chain().any(|cause| cause.downcast_ref::<std::io::Error>().is_some()) {
        IrcStage::SendLineReached
    } else {
        IrcStage::BeforeSendLine
    }
}

/// Whether a send that failed on both halves may have reached Twitch.
pub(crate) fn may_have_left(helix: HelixStage, irc: IrcStage) -> bool {
    irc == IrcStage::SendLineReached || helix == HelixStage::AfterLeft
}

/// Whether a linked account's send may have posted, so its limiter counts
/// it: sent, an unreadable 2xx (read as not sent without a reason), a 5xx,
/// or any error after the request left.
pub(crate) fn alt_send_may_have_posted(result: &anyhow::Result<(Option<String>, bool, Option<String>)>) -> bool {
    match result {
        Ok((_, true, _)) => true,
        Ok((_, false, reason)) => reason.as_deref().is_none_or(|r| r.trim().is_empty()),
        Err(e) => helix_stage(e) == HelixStage::AfterLeft,
    }
}

/// Whether a refusal locks the main account's message box behind a warning.
/// Only the main account's: a warned linked account never flags it.
pub(crate) fn warns_primary(is_primary: bool, drop_reason: Option<&str>) -> bool {
    is_primary && crate::services::chat_warning::is_warned_refusal(None, drop_reason)
}

/// How the main account's Helix try went.
pub(crate) enum HelixTry {
    Sent(Option<String>),
    /// Twitch accepted the request and dropped the message, with its reason.
    Dropped(Option<String>),
    /// Fall to IRC; the stage the Helix try reached.
    FallBack(HelixStage),
}

/// The main account's Helix try: the token first (no token falls to IRC as
/// `NoToken`), then the send.
pub(crate) async fn try_primary_helix<T, TF, S, SF>(token: T, send: S) -> HelixTry
where
    T: FnOnce() -> TF,
    TF: Future<Output = anyhow::Result<String>>,
    S: FnOnce(String) -> SF,
    SF: Future<Output = anyhow::Result<(Option<String>, bool, Option<String>)>>,
{
    let token = match token().await {
        Ok(token) => token,
        Err(e) => {
            log::warn!("[Chat] No token for the Helix send ({}); falling back to IRC", e);
            return HelixTry::FallBack(HelixStage::NoToken);
        }
    };
    match send(token).await {
        Ok((message_id, true, _)) => HelixTry::Sent(message_id),
        Ok((_, false, drop_reason)) => HelixTry::Dropped(drop_reason),
        Err(e) => {
            log::warn!("[Chat] Helix send failed ({}); falling back to IRC", e);
            HelixTry::FallBack(helix_stage(&e))
        }
    }
}

/// Twitch's answer to an automation's send, read without defaults: only an
/// explicit `data[0].is_sent` decides Sent or Refused. The flag is true for a
/// refusal Twitch gave in a 2xx body with its own reason.
async fn read_engine_answer(response: reqwest::Response) -> (SendOutcome, bool) {
    let status = response.status();
    // A 5xx or a 3xx (never followed) may have posted: Unknown, the claim stays.
    if status.is_server_error() || status.is_redirection() {
        return (SendOutcome::Unknown, false);
    }
    if status.as_u16() == 401 {
        return (SendOutcome::Refused(format!("{UNAUTHORIZED} (401)")), false);
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let reason = crate::services::twitch_service::helix_refusal_text(status.as_u16(), &body);
        return (SendOutcome::Refused(reason), false);
    }
    let Ok(bytes) = response.bytes().await else { return (SendOutcome::Unknown, false) };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return (SendOutcome::Unknown, false);
    };
    match json.pointer("/data/0/is_sent").and_then(serde_json::Value::as_bool) {
        Some(true) => {
            let message_id = json
                .pointer("/data/0/message_id")
                .and_then(|v| v.as_str())
                .filter(|id| !id.is_empty())
                .map(str::to_string);
            (SendOutcome::Sent { message_id }, false)
        }
        Some(false) => {
            let reason = json
                .pointer("/data/0/drop_reason/message")
                .or_else(|| json.pointer("/data/0/drop_reason/code"))
                .and_then(|v| v.as_str())
                .filter(|r| !r.trim().is_empty())
                .unwrap_or("Twitch did not accept the message")
                .to_string();
            (SendOutcome::Refused(reason), true)
        }
        None => (SendOutcome::Unknown, false),
    }
}

/// The automation send door over a given client and address (tests use a
/// local fake). Marks the ticket sending right before the request, records
/// it on Sent or Unknown and cancels it on Refused. No retry, no IRC.
pub(crate) async fn send_helix_only_via(
    client: &reqwest::Client,
    url: &str,
    ticket: &mut Ticket,
    token: &str,
    text: &str,
    parent: Option<&str>,
    login: &str,
    room_id: &str,
    send_as: &str,
) -> SendOutcome {
    if token.trim().is_empty() {
        chat_send_limiter::cancel_booking(ticket);
        return SendOutcome::Refused("Sign in to Twitch to send messages".into());
    }
    let mut body = serde_json::json!({ "broadcaster_id": room_id, "sender_id": send_as, "message": text });
    if let Some(parent) = parent.filter(|id| !id.starts_with("local-")) {
        body["reply_parent_message_id"] = serde_json::Value::String(parent.to_string());
    }
    let request = client
        .post(url)
        .bearer_auth(token)
        .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
        .json(&body);
    chat_send_limiter::mark_sending(ticket);
    let (outcome, twitch_reason) = match request.send().await {
        Err(e) if e.is_connect() || e.is_builder() => (SendOutcome::Refused(format!("Couldn't reach Twitch ({e})")), false),
        Err(_) => (SendOutcome::Unknown, false),
        Ok(response) => read_engine_answer(response).await,
    };
    match &outcome {
        SendOutcome::Refused(reason) => {
            chat_send_limiter::cancel_booking(ticket);
            if twitch_reason && warns_primary(ticket.account().is_primary(), Some(reason)) {
                let (channel, reason) = (login.to_string(), reason.clone());
                tokio::spawn(async move {
                    crate::services::chat_warning::on_refusal(&channel, None, Some(&reason)).await;
                });
            }
        }
        SendOutcome::Sent { .. } | SendOutcome::Unknown => chat_send_limiter::record_ticket(ticket),
    }
    outcome
}

/// How long a user's stop waits for a chat start already running. Bounded: a
/// start stuck on the network (the HTTP client's 30 s timeout) must not hold
/// a close.
const STOP_WAITS_FOR_START: Duration = Duration::from_secs(3);

/// Result of a send. `message_id` is Twitch's authoritative id when the message
/// went out over Helix (so the optimistic copy can be stamped and deleted
/// without waiting on the IRC echo); it is None on the IRC fallback path
/// (slash-commands, or Helix unavailable). `is_sent` is false when Twitch
/// dropped the message (e.g. AutoMod), with `drop_reason` set.
#[derive(serde::Serialize)]
pub struct SendResult {
    pub message_id: Option<String>,
    pub is_sent: bool,
    pub drop_reason: Option<String>,
}

pub struct ChatService;

impl ChatService {
    pub async fn start(
        channel: &str,
        state: &AppState,
        claim: bool,
        reattach: bool,
        window: &str,
    ) -> Result<u16> {
        IrcService::start(channel, state, claim, reattach, window).await
    }

    pub async fn send_message(
        message: &str,
        reply_parent_msg_id: Option<&str>,
        target_channel: Option<&str>,
        broadcaster_id: Option<&str>,
        sender_id: Option<&str>,
        sender_account_id: Option<&str>,
    ) -> std::result::Result<SendResult, SendError> {
        // Our own rows carry a local id until Twitch's id is stamped onto them.
        // Twitch refuses a reply to a parent id it never issued ("Message
        // cannot be replied to"), so such a reply goes out as a plain message.
        let reply_parent_msg_id = reply_parent_msg_id.filter(|id| !id.starts_with("local-"));

        // Slash-commands (/ban, /me, /timeout, ...) MUST go over IRC so Twitch
        // executes them. Helix Send Chat Message would post them as literal text.
        let is_command = message.trim_start().starts_with('/');

        // Is this being sent from a NON-primary (secondary) account? The primary
        // owns the IRC connection and the cached token; secondaries send via Helix
        // with their own token only.
        let primary_id = AccountStore::primary().map(|p| p.user_id);
        let is_secondary = match (sender_account_id, primary_id.as_deref()) {
            (Some(acc), Some(pid)) => acc != pid,
            (Some(_), None) => true,
            _ => false,
        };

        if is_secondary {
            let account_id = sender_account_id.unwrap();

            // Slash-commands need the account's own IRC connection, which only the
            // primary has. Don't silently post them as literal text from an alt.
            if is_command {
                return Ok(SendResult {
                    message_id: None,
                    is_sent: false,
                    drop_reason: Some(
                        "Slash commands can only be sent from your main account.".to_string(),
                    ),
                });
            }

            let bid = broadcaster_id.ok_or_else(|| {
                SendError::before(anyhow::anyhow!("broadcaster_id required to send from a secondary account"))
            })?;
            // The account id IS the Twitch user id; prefer an explicit sender_id.
            let sid = sender_id.unwrap_or(account_id);
            let token = AccountStore::get_token_for(account_id).await.map_err(SendError::before)?;

            let result = TwitchService::send_chat_message_helix_with_token(
                &token,
                bid,
                sid,
                message,
                reply_parent_msg_id,
            )
            .await;
            // The linked account's limiter counts what may have posted, so an
            // automation sending as it keeps its spacing after this message.
            if alt_send_may_have_posted(&result) {
                if let Some(ch) = target_channel {
                    chat_send_limiter::record_sent_as(account_id, ch, message);
                }
            }
            return match result {
                Ok((message_id, true, _)) => Ok(SendResult {
                    message_id,
                    is_sent: true,
                    drop_reason: None,
                }),
                Ok((_, false, drop_reason)) => Ok(SendResult {
                    message_id: None,
                    is_sent: false,
                    drop_reason,
                }),
                // No IRC fallback for secondaries: the IRC connection belongs to the
                // primary, so falling back would send the message from the wrong
                // account. Surface the failure instead.
                Err(e) => {
                    log::warn!("[Chat] send from account {} failed: {}", account_id, e);
                    Ok(SendResult {
                        message_id: None,
                        is_sent: false,
                        drop_reason: Some(format!("Could not send from this account: {}", e)),
                    })
                }
            };
        }

        // ----- PRIMARY path (unchanged behavior) -----
        // The token is fetched first, so a token that cannot be had (a refresh
        // timeout included) falls to IRC as "no token": nothing left the device.
        let mut helix = HelixStage::NotTried;
        if !is_command {
            if let (Some(bid), Some(sid)) = (broadcaster_id, sender_id) {
                let attempt = try_primary_helix(TwitchService::get_token, |token| async move {
                    TwitchService::send_chat_message_helix_with_token(&token, bid, sid, message, reply_parent_msg_id).await
                })
                .await;
                match attempt {
                    HelixTry::Sent(message_id) => {
                        if let Some(ch) = target_channel {
                            crate::services::chat_send_limiter::record_sent(ch, message);
                        }
                        // Authoritative id straight from the send; deletes no
                        // longer depend on catching the IRC echo.
                        return Ok(SendResult {
                            message_id,
                            is_sent: true,
                            drop_reason: None,
                        });
                    }
                    HelixTry::Dropped(drop_reason) => {
                        // Twitch accepted the request but dropped the message
                        // (AutoMod, etc.). Report it; do NOT also send over IRC.
                        if let Some(ch) = target_channel.filter(|_| {
                            crate::services::chat_warning::is_warned_refusal(None, drop_reason.as_deref())
                        }) {
                            let (ch, text) = (ch.to_string(), drop_reason.clone());
                            tokio::spawn(async move {
                                crate::services::chat_warning::on_refusal(&ch, None, text.as_deref()).await;
                            });
                        }
                        return Ok(SendResult {
                            message_id: None,
                            is_sent: false,
                            drop_reason,
                        });
                    }
                    HelixTry::FallBack(stage) => {
                        // fall through to the IRC path below
                        helix = stage;
                    }
                }
            }
        }

        // IRC path: slash-commands, or Helix unavailable / errored. No id here, so
        // the optimistic copy keeps its local id and relies on echo matching.
        if let Err(e) = IrcService::send_message(message, reply_parent_msg_id, target_channel).await {
            let may_have_left = may_have_left(helix, irc_stage(&e));
            return Err(SendError { source: e, may_have_left });
        }
        if let Some(ch) = target_channel {
            crate::services::chat_send_limiter::record_sent(ch, message);
        }
        Ok(SendResult {
            message_id: None,
            is_sent: true,
            drop_reason: None,
        })
    }

    /// An automation's chat message: Helix only, on the automation HTTP
    /// client, on its booked ticket. The caller fetched the token and the
    /// room id and ran the send-time checks right before; this marks the
    /// ticket sending, sends, and settles the ticket (recorded on Sent or
    /// Unknown, cancelled on Refused). Never falls back to IRC, never
    /// retries.
    pub async fn send_message_helix_only(
        ticket: &mut Ticket,
        token: &str,
        text: &str,
        parent: Option<&str>,
        login: &str,
        room_id: &str,
        send_as: &str,
    ) -> SendOutcome {
        send_helix_only_via(&ENGINE_CLIENT, HELIX_CHAT_URL, ticket, token, text, parent, login, room_id, send_as).await
    }

    /// Stop chat because the USER is done with it (stopped a stream, `/disconnect`).
    ///
    /// A full `IrcService::stop()` aborts the shared local-WS bridge, which every
    /// Kick/YouTube/TikTok pane in every window is also riding. So when providers
    /// still hold the bridge, clear only the Twitch IRC half — the same condition
    /// `IrcService::start` already uses before choosing `stop_irc_only`.
    ///
    /// Recovery from a WEDGED backend is a different intent and must NOT come
    /// through here: it needs the unconditional teardown. See `restart_bridge`.
    ///
    /// `window` is the caller. While another window still holds a Twitch channel
    /// (a floating chat, a MultiChat window) the connection is theirs too and
    /// stays up; the caller's own channels PART through its releases. The same
    /// goes for a room the viewer keeps open beside any stream, a docked chat
    /// on desktop or a kept chat on the phones' Chats screen: closing the
    /// player must not take its connection away while it is still on screen.
    /// With providers on the bridge the stop would be Twitch-only and leave
    /// every socket open, so no store would notice and claim again: then any
    /// channel the caller holds keeps the connection up as well. A full stop
    /// closes the sockets and each store re-claims on reconnect.
    ///
    /// Returns whether the connection actually went down (false = kept).
    pub async fn stop(window: &str) -> Result<bool> {
        // A stop that lands mid-start tore the bridge from under a connection
        // still being built (closing a chat moments after opening it), so it
        // waits for that start first, briefly.
        let _start_guard = match tokio::time::timeout(
            STOP_WAITS_FOR_START,
            crate::services::irc_service::get_start_lock().lock(),
        )
        .await
        {
            Ok(guard) => Some(guard),
            Err(_) => {
                crate::services::irc_service::record_lifecycle(&format!(
                    "stop_chat from {}: a start still running after {} s; stopping anyway",
                    window,
                    STOP_WAITS_FOR_START.as_secs()
                ));
                None
            }
        };
        let providers = crate::services::providers::has_active_bridge_users();
        let other_windows = IrcService::held_by_other_windows(window).await;
        let held = IrcService::channels_held_by(window).await;
        let kept_for = stop_kept_for(other_windows, providers, &held, &kept_rooms());
        if let Some(why) = kept_for {
            crate::services::irc_service::record_lifecycle(&format!(
                "stop_chat from {} kept the connection: {}",
                window, why
            ));
            return Ok(false);
        }
        // Nothing but the engine keeps the connection: it stays up for the
        // engine's rooms, the caller's claims go and every other room PARTs.
        // Providers need no special case: with them on the bridge
        // `stop_kept_for` already kept the connection for a caller that held a
        // Twitch room, so only a caller that held none gets here and sockets
        // stay as they are.
        if IrcService::held_by_window(crate::services::account_automations::AUTOMATIONS_LABEL).await {
            IrcService::stop_keeping_engine_rooms(window).await;
            return Ok(true);
        }
        if providers {
            crate::services::irc_service::record_lifecycle(&format!(
                "stop_chat from {}: Twitch side stopped, the bridge stays for other platforms",
                window
            ));
            IrcService::stop_twitch_only().await;
            return Ok(true);
        }
        crate::services::irc_service::record_lifecycle(&format!(
            "stop_chat from {}: full stop (held {:?})",
            window, held
        ));
        IrcService::stop().await.map(|_| true)
    }

    /// Force the one true teardown, bridge included, regardless of provider
    /// consumers. The chat watchdog escalates here when two reconnects have failed
    /// to revive a wedged task and `start_chat`'s idempotent path cannot fix it.
    /// Provider slices are re-established by the frontend's own `reconnectAll`.
    pub async fn restart_bridge() -> Result<()> {
        IrcService::stop().await
    }

    pub async fn join_channel(channel: &str, state: &AppState, window: &str) -> Result<()> {
        IrcService::join_channel(channel, window).await?;
        // Populate the per-channel emote cache for the newly-JOINed channel.
        // Without this, `parse_text_segment` can't find 7TV/FFZ/BTTV emotes
        // for messages from this channel (it reads from CHANNEL_EMOTES which
        // is keyed per channel). Previously every `start_chat` did a full
        // tear-down + re-fetch which masked the gap; with idempotent `start`,
        // additional-channel JOINs (channel switching, MultiChat tabs, etc.)
        // now route through this path.
        // Deferred: this runs on channel switches and MultiChat tab opens, where
        // a user is waiting on the pane. The disk seed lands before we return;
        // the provider refresh follows.
        IrcService::seed_emotes_deferring_refresh(channel, state.emote_service.clone()).await;
        Ok(())
    }

    pub async fn leave_channel(channel: &str, window: &str) -> Result<()> {
        IrcService::leave_channel(channel, window).await
    }
}

/// Why the user's stop leaves the shared connection up, or `None` to take it down.
///
/// `held` is what the calling window holds (Twitch channel keys); `kept` names
/// the Twitch rooms the viewer keeps open beside any stream. A kept room counts
/// only while the caller actually holds it: one restored but never joined has
/// nothing riding the connection.
fn stop_kept_for(other_windows: bool, providers: bool, held: &[String], kept: &[String]) -> Option<&'static str> {
    if other_windows {
        return Some("another window holds a channel");
    }
    if held.iter().any(|c| kept.contains(c)) {
        return Some("a chat kept open beside the stream holds a channel");
    }
    if providers && !held.is_empty() {
        return Some("this window still holds a channel");
    }
    None
}

/// The Twitch rooms the viewer keeps open beside any stream, by channel key: a
/// kept chat on the phones' Chats screen (the stream's own tab only when kept)
/// and a docked chat on desktop.
fn kept_rooms() -> Vec<String> {
    #[allow(unused_mut)]
    let mut rooms: Vec<String> = crate::services::chat_tabs::joined()
        .into_iter()
        .filter(|t| t.kept && t.provider == "twitch")
        .map(|t| t.login.trim().to_lowercase())
        .collect();
    #[cfg(desktop)]
    rooms.extend(
        crate::commands::chat_dock::get_chat_dock()
            .chats
            .into_iter()
            .filter(|c| c.provider == "twitch")
            .map(|c| c.login.trim().to_lowercase()),
    );
    rooms
}

#[cfg(test)]
mod engine_stop_tests {
    fn stop_body() -> String {
        let src = include_str!("chat_service.rs").replace("\r\n", "\n");
        let start = src.find("pub async fn stop(window: &str)").expect("stop");
        src[start..start + src[start..].find("\n    }\n").expect("end")].to_string()
    }

    #[test]
    fn the_kept_decision_runs_before_the_engine_only_branch() {
        let b = stop_body();
        let kept = b.find("stop_kept_for(").expect("kept decision");
        let engine = b.find("stop_keeping_engine_rooms(").expect("engine branch");
        let providers = b.find("if providers {").expect("provider branch");
        assert!(kept < engine && engine < providers);
        assert!(b.contains("held_by_other_windows(window)"));
    }
}

#[cfg(test)]
mod stop_decision_tests {
    use super::stop_kept_for;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn closing_the_player_keeps_a_kept_chats_room() {
        // The phone case: the stream's room is also one of your chats, so the
        // Chats screen still shows it after the player closes.
        let why = stop_kept_for(false, false, &keys(&["kathi"]), &keys(&["kathi"]));
        assert!(why.is_some());
    }

    #[test]
    fn a_kept_room_beside_another_stream_keeps_the_connection() {
        let why = stop_kept_for(false, false, &keys(&["nicewigg", "summit1g"]), &keys(&["summit1g"]));
        assert!(why.is_some());
    }

    #[test]
    fn a_docked_chat_keeps_the_connection_without_other_platforms() {
        let why = stop_kept_for(false, false, &keys(&["xqc", "lirik"]), &keys(&["lirik"]));
        assert_eq!(why, Some("a chat kept open beside the stream holds a channel"));
    }

    #[test]
    fn the_stream_chat_alone_goes_down_with_the_stream() {
        assert_eq!(stop_kept_for(false, false, &keys(&["xqc"]), &[]), None);
        assert_eq!(stop_kept_for(false, false, &[], &[]), None);
    }

    #[test]
    fn a_kept_room_that_was_never_joined_holds_nothing() {
        assert_eq!(stop_kept_for(false, false, &keys(&["xqc"]), &keys(&["lirik"])), None);
    }

    #[test]
    fn another_window_always_keeps_it() {
        assert_eq!(stop_kept_for(true, false, &[], &[]), Some("another window holds a channel"));
    }

    #[test]
    fn with_other_platforms_on_the_bridge_any_held_channel_keeps_it() {
        assert_eq!(stop_kept_for(false, true, &keys(&["xqc"]), &[]), Some("this window still holds a channel"));
        assert_eq!(stop_kept_for(false, true, &[], &[]), None);
    }
}

#[cfg(test)]
mod send_outcome_tests {
    use super::*;
    use crate::services::chat_send_limiter::{acquire_within_on, alt_entries, LimiterKey, MAX_WAIT};
    use std::sync::atomic::{AtomicBool, Ordering};

    async fn closed_port_error() -> reqwest::Error {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        reqwest::Client::new().post(format!("http://127.0.0.1:{port}/")).send().await.unwrap_err()
    }

    #[test]
    fn may_have_left_follows_the_table() {
        use HelixStage::*;
        use IrcStage::*;
        for helix in [NoToken, BeforeLeft, Refused(401), Refused(403), Refused(400), Refused(422), NotTried] {
            assert!(!may_have_left(helix, BeforeSendLine), "{helix:?} with IRC down");
        }
        assert!(may_have_left(AfterLeft, BeforeSendLine));
        for helix in [NoToken, BeforeLeft, Refused(401), NotTried, AfterLeft] {
            assert!(may_have_left(helix, SendLineReached), "{helix:?} with a partial IRC write");
        }
    }

    #[tokio::test]
    async fn helix_stage_reads_real_errors() {
        let closed = closed_port_error().await;
        assert!(closed.is_connect());
        assert_eq!(helix_stage(&anyhow::Error::from(closed)), HelixStage::BeforeLeft);
        let bad = reqwest::Client::new().post("not a url").send().await.unwrap_err();
        assert!(bad.is_builder());
        assert_eq!(helix_stage(&anyhow::Error::from(bad)), HelixStage::BeforeLeft);
        let refused = anyhow::Error::from(HelixSendRefused { status: 401, body: serde_json::Value::Null });
        assert_eq!(helix_stage(&refused), HelixStage::Refused(401));
        let failed = anyhow::Error::from(HelixSendRefused { status: 503, body: serde_json::Value::Null });
        assert_eq!(helix_stage(&failed), HelixStage::AfterLeft);
        let moved = anyhow::Error::from(HelixSendRefused { status: 302, body: serde_json::Value::Null });
        assert_eq!(helix_stage(&moved), HelixStage::AfterLeft, "a 3xx is never an answered refusal");
        assert_eq!(helix_stage(&anyhow::anyhow!("error decoding response body")), HelixStage::AfterLeft);
    }

    #[test]
    fn irc_stage_reads_where_the_send_failed() {
        let pipe = anyhow::Error::from(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe"));
        assert_eq!(irc_stage(&pipe), IrcStage::SendLineReached);
        assert_eq!(irc_stage(&pipe.context("writing PRIVMSG")), IrcStage::SendLineReached);
        let rejoin = anyhow::anyhow!("Failed to re-JOIN channel {} before send: {}", "xqc", "timed out");
        assert_eq!(irc_stage(&rejoin), IrcStage::BeforeSendLine);
        assert_eq!(irc_stage(&anyhow::anyhow!("IRC connection not established")), IrcStage::BeforeSendLine);
    }

    #[test]
    fn a_send_errors_text_is_the_text_callers_always_saw() {
        for text in ["IRC connection not established", "broadcaster_id required to send from a secondary account"] {
            assert_eq!(SendError::before(anyhow::anyhow!(text)).to_string(), anyhow::anyhow!(text).to_string());
        }
        let wrapped = anyhow::Error::from(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe")).context("writing");
        let today = wrapped.to_string();
        assert_eq!(SendError { source: wrapped, may_have_left: true }.to_string(), today);
    }

    #[tokio::test]
    async fn no_token_falls_to_irc_as_no_token() {
        let sent = AtomicBool::new(false);
        let sent_ref = &sent;
        let out = try_primary_helix(
            || async { Err::<String, _>(anyhow::anyhow!("Token refresh was interrupted (timed out)")) },
            move |_token| async move {
                sent_ref.store(true, Ordering::SeqCst);
                Ok((None, true, None))
            },
        )
        .await;
        assert!(matches!(out, HelixTry::FallBack(HelixStage::NoToken)));
        assert!(!sent.load(Ordering::SeqCst), "no send without a token");
        let refused = try_primary_helix(
            || async { Ok("t".to_string()) },
            |_token| async { Err(HelixSendRefused { status: 422, body: serde_json::Value::Null }.into()) },
        )
        .await;
        assert!(matches!(refused, HelixTry::FallBack(HelixStage::Refused(422))));
        let ok = try_primary_helix(|| async { Ok("t".to_string()) }, |_token| async { Ok((Some("m".into()), true, None)) }).await;
        assert!(matches!(ok, HelixTry::Sent(Some(id)) if id == "m"));
        let dropped = try_primary_helix(|| async { Ok("t".to_string()) }, |_token| async { Ok((None, false, Some("automod".into()))) }).await;
        assert!(matches!(dropped, HelixTry::Dropped(Some(reason)) if reason == "automod"));
    }

    #[tokio::test]
    async fn a_linked_accounts_send_counts_when_it_may_have_posted() {
        assert!(alt_send_may_have_posted(&Ok((Some("m".into()), true, None))));
        assert!(alt_send_may_have_posted(&Ok((None, false, None))), "an unreadable 2xx");
        assert!(!alt_send_may_have_posted(&Ok((None, false, Some("Your message is identical".into())))));
        assert!(alt_send_may_have_posted(&Err(HelixSendRefused { status: 503, body: serde_json::Value::Null }.into())));
        assert!(!alt_send_may_have_posted(&Err(HelixSendRefused { status: 403, body: serde_json::Value::Null }.into())));
        assert!(alt_send_may_have_posted(&Err(anyhow::anyhow!("error decoding response body"))));
        assert!(!alt_send_may_have_posted(&Err(closed_port_error().await.into())));
    }

    #[test]
    fn only_the_main_accounts_warned_refusal_locks_its_box() {
        let warned = "You must acknowledge the warning before you can chat";
        assert!(warns_primary(true, Some(warned)));
        assert!(!warns_primary(false, Some(warned)));
        assert!(!warns_primary(true, Some("Your message is identical")));
    }

    fn request_complete(seen: &[u8]) -> bool {
        let text = String::from_utf8_lossy(seen);
        let Some(head_end) = text.find("\r\n\r\n") else { return false };
        let length = text[..head_end]
            .lines()
            .find_map(|line| {
                let (k, v) = line.split_once(':')?;
                k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok()).flatten()
            })
            .unwrap_or(0);
        seen.len() >= head_end + 4 + length
    }

    /// A one-request HTTP server on 127.0.0.1: it reads the request and
    /// answers `reply`, or holds the connection without answering.
    async fn fake_helix(reply: Option<String>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = sock.read(&mut buf).await else { return };
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                if request_complete(&seen) {
                    break;
                }
            }
            match reply {
                Some(reply) => {
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                }
                None => tokio::time::sleep(Duration::from_secs(30)).await,
            }
        });
        format!("http://{addr}/helix/chat/messages")
    }

    /// A listener that accepts TCP and never answers, so a TLS handshake stalls.
    async fn silent_tls() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        format!("https://{addr}/helix/chat/messages")
    }

    fn http(status: &str, body: &str) -> Option<String> {
        Some(format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    fn quick() -> reqwest::Client {
        engine_client(Duration::from_millis(300), Duration::from_millis(800))
    }

    /// One send through the door on its own linked account: the outcome and
    /// that account's (entries, booked slots) afterwards.
    async fn door(url: &str, client: &reqwest::Client, name: &str) -> (SendOutcome, (usize, usize)) {
        let id = format!("door-{name}");
        let (_, mut ticket) = acquire_within_on(LimiterKey::Alt(id.clone()), "doorchan", "hello", MAX_WAIT).expect("a slot");
        let out = send_helix_only_via(client, url, &mut ticket, "token", "hello", None, "doorchan", "42", "7").await;
        assert!(ticket.is_settled());
        drop(ticket);
        (out, alt_entries(&id))
    }

    #[tokio::test]
    async fn the_engine_door_reads_twitchs_answer_itself() {
        let c = quick();
        let sent = http("200 OK", r#"{"data":[{"message_id":"m1","is_sent":true}]}"#);
        assert_eq!(door(&fake_helix(sent).await, &c, "sent").await, (SendOutcome::Sent { message_id: Some("m1".into()) }, (1, 0)));
        let refused = http(
            "200 OK",
            r#"{"data":[{"message_id":"","is_sent":false,"drop_reason":{"code":"msg_duplicate","message":"Your message is identical to the previous one"}}]}"#,
        );
        assert_eq!(
            door(&fake_helix(refused).await, &c, "refused").await,
            (SendOutcome::Refused("Your message is identical to the previous one".into()), (0, 0))
        );
        assert_eq!(door(&fake_helix(http("200 OK", "not json")).await, &c, "unreadable").await, (SendOutcome::Unknown, (1, 0)));
        let no_field = http("200 OK", r#"{"data":[{"message_id":"m1"}]}"#);
        assert_eq!(door(&fake_helix(no_field).await, &c, "no-field").await, (SendOutcome::Unknown, (1, 0)));
        let server = http("503 Service Unavailable", "{}");
        assert_eq!(door(&fake_helix(server).await, &c, "server").await, (SendOutcome::Unknown, (1, 0)));
        let auth = http("401 Unauthorized", r#"{"error":"Unauthorized","status":401,"message":"Invalid OAuth token"}"#);
        let (out, entries) = door(&fake_helix(auth).await, &c, "auth").await;
        assert!(matches!(&out, SendOutcome::Refused(reason) if is_unauthorized(reason)), "{out:?}");
        assert_eq!(entries, (0, 0));
        assert!(!is_unauthorized("Your message is identical to the previous one"));
    }

    #[tokio::test]
    async fn a_send_that_never_answers_is_unknown_at_the_total_timeout() {
        let started = std::time::Instant::now();
        assert_eq!(door(&fake_helix(None).await, &quick(), "silent").await, (SendOutcome::Unknown, (1, 0)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    /// A server that answers every request 302 to `/moved` and records the
    /// request line of each request it saw.
    async fn fake_redirect() -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen_paths = paths.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let seen_paths = seen_paths.clone();
                tokio::spawn(async move {
                    let mut seen = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let Ok(n) = sock.read(&mut buf).await else { return };
                        if n == 0 {
                            return;
                        }
                        seen.extend_from_slice(&buf[..n]);
                        if request_complete(&seen) {
                            break;
                        }
                    }
                    let first = String::from_utf8_lossy(&seen).lines().next().unwrap_or_default().to_string();
                    seen_paths.lock().unwrap().push(first);
                    let reply = "HTTP/1.1 302 Found\r\nLocation: /moved\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (format!("http://{addr}/helix/chat/messages"), paths)
    }

    #[tokio::test]
    async fn a_redirect_is_never_followed_and_reads_unknown() {
        let (url, paths) = fake_redirect().await;
        let (out, entries) = door(&url, &quick(), "redirect").await;
        assert_eq!(out, SendOutcome::Unknown, "a 3xx may have posted: the claim stays");
        assert_eq!(entries, (1, 0), "counted like an Unknown");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let seen = paths.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].starts_with("POST /helix/chat/messages"), "{seen:?}");
        assert!(!seen.iter().any(|line| line.contains("/moved")), "zero requests to the Location: {seen:?}");
    }

    #[tokio::test]
    async fn a_connect_stall_is_refused() {
        let url = silent_tls().await;
        let client = engine_client(Duration::from_millis(300), Duration::from_secs(3));
        let raw = client.post(&url).body("x").send().await.unwrap_err();
        assert!(raw.is_connect(), "a connect timeout reports is_connect: {raw:?}");
        let (out, entries) = door(&url, &client, "stall").await;
        assert!(matches!(out, SendOutcome::Refused(_)), "{out:?}");
        assert_eq!(entries, (0, 0));
    }

    #[tokio::test]
    async fn a_closed_port_and_an_empty_token_send_nothing() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (out, entries) = door(&format!("http://127.0.0.1:{port}/"), &quick(), "closed").await;
        assert!(matches!(out, SendOutcome::Refused(_)), "{out:?}");
        assert_eq!(entries, (0, 0));
        let (_, mut ticket) = acquire_within_on(LimiterKey::Alt("door-empty".into()), "doorchan", "hello", MAX_WAIT).unwrap();
        let out = send_helix_only_via(&quick(), "http://127.0.0.1:9/", &mut ticket, " ", "hello", None, "doorchan", "42", "7").await;
        assert_eq!(out, SendOutcome::Refused("Sign in to Twitch to send messages".into()));
        drop(ticket);
        assert_eq!(alt_entries("door-empty"), (0, 0));
    }

    #[test]
    fn the_engine_door_fetches_no_token_and_counts_only_through_its_ticket() {
        let src = include_str!("chat_service.rs").replace("\r\n", "\n");
        let door = src.split("pub async fn send_message_helix_only(").nth(1).unwrap().split("\n    }\n").next().unwrap();
        let via = src.split("pub(crate) async fn send_helix_only_via(").nth(1).unwrap().split("\n}\n").next().unwrap();
        for body in [door, via] {
            assert!(!body.contains("get_token"));
            assert!(!body.contains("record_sent"));
            assert!(!body.contains("IrcService"));
        }
        assert!(via.contains("chat_send_limiter::record_ticket(ticket)"));
        assert!(via.contains("chat_send_limiter::mark_sending(ticket);\n    let (outcome, twitch_reason) = match request.send().await"));
        assert!(src.contains("engine_client(ENGINE_CONNECT_TIMEOUT, ENGINE_SEND_TIMEOUT)"));
        let build = src.split("pub(crate) fn engine_client(").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(build.contains(".redirect(reqwest::redirect::Policy::none())"), "the engine client follows no redirect");
        assert_eq!(ENGINE_CONNECT_TIMEOUT, Duration::from_secs(3));
        assert_eq!(ENGINE_SEND_TIMEOUT, Duration::from_secs(5));
    }
}
