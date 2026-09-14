// This module now uses IRC for chat instead of EventSub
// Simply re-exports the IrcService as ChatService for backwards compatibility

use crate::models::settings::AppState;
use crate::services::account_store::AccountStore;
use crate::services::irc_service::IrcService;
use crate::services::twitch_service::TwitchService;
use anyhow::Result;

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
    pub async fn stop() -> Result<()> {
        if crate::services::providers::has_active_bridge_users() {
            IrcService::stop_twitch_only().await;
            return Ok(());
        }
        IrcService::stop().await
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
