// This module now uses IRC for chat instead of EventSub
// Simply re-exports the IrcService as ChatService for backwards compatibility

use crate::models::settings::AppState;
use crate::services::account_store::AccountStore;
use crate::services::irc_service::IrcService;
use crate::services::twitch_service::TwitchService;
use anyhow::Result;
use std::time::Duration;

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
    ) -> Result<SendResult> {
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
                anyhow::anyhow!("broadcaster_id required to send from a secondary account")
            })?;
            // The account id IS the Twitch user id; prefer an explicit sender_id.
            let sid = sender_id.unwrap_or(account_id);
            let token = AccountStore::get_token_for(account_id).await?;

            return match TwitchService::send_chat_message_helix_with_token(
                &token,
                bid,
                sid,
                message,
                reply_parent_msg_id,
            )
            .await
            {
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
                Err(e) => Ok(SendResult {
                    message_id: None,
                    is_sent: false,
                    drop_reason: Some(format!("Could not send from this account: {}", e)),
                }),
            };
        }

        // ----- PRIMARY path (unchanged behavior) -----
        if !is_command {
            if let (Some(bid), Some(sid)) = (broadcaster_id, sender_id) {
                match TwitchService::send_chat_message_helix(bid, sid, message, reply_parent_msg_id)
                    .await
                {
                    Ok((message_id, true, _)) => {
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
                    Ok((_, false, drop_reason)) => {
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
                    Err(e) => {
                        log::warn!("[Chat] Helix send failed ({}); falling back to IRC", e);
                        // fall through to the IRC path below
                    }
                }
            }
        }

        // IRC path: slash-commands, or Helix unavailable / errored. No id here, so
        // the optimistic copy keeps its local id and relies on echo matching.
        IrcService::send_message(message, reply_parent_msg_id, target_channel).await?;
        if let Some(ch) = target_channel {
            crate::services::chat_send_limiter::record_sent(ch, message);
        }
        Ok(SendResult {
            message_id: None,
            is_sent: true,
            drop_reason: None,
        })
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
