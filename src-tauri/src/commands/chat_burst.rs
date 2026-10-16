//! `/spam` and `/pyramid` (services::chat_burst). The chat box hands the raw
//! arguments over; parsing, the lines, the pacing and the stop rules all live
//! in Rust.

use serde::Serialize;
use tauri::{Emitter, State};

use crate::models::settings::AppState;
use crate::services::account_store::AccountStore;
use crate::services::chat_burst::{self, BurstArgs, BurstKind, BurstStatus, Job};
use crate::services::twitch_service::{HelixSendRefused, TwitchService};

/// What the chat box shows after a `/spam` or `/pyramid`.
#[derive(Serialize)]
pub struct ChatBurstReply {
    /// One line for the chat box to show.
    pub message: String,
    /// The run now going in this channel, if any.
    pub status: Option<BurstStatus>,
}

/// Runs `/spam <args>` or `/pyramid <args>` in a Twitch channel: starts a run,
/// or stops the one going there. A refusal comes back as Err with the reason.
#[tauri::command]
pub async fn run_chat_burst(
    kind: String,
    args: String,
    channel_login: String,
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<ChatBurstReply, String> {
    let settings = state
        .settings
        .lock()
        .map(|s| (s.chat_burst.delay_ms, s.chat_burst.vary_repeats))
        .map_err(|_| "settings unavailable".to_string())?;
    start_burst(&kind, &args, &channel_login, &channel_id, settings, false).await
}

/// Start or stop a run, for the command above and the slash dispatcher.
/// `settings` is (delay_ms, vary_repeats) from Settings. With `announce_end`
/// the way the run ends (done, stopped, failed) is printed in the room's chat,
/// for a phone, which shows no progress bar.
pub(crate) async fn start_burst(
    kind: &str,
    args: &str,
    channel_login: &str,
    channel_id: &str,
    settings: (u32, bool),
    announce_end: bool,
) -> Result<ChatBurstReply, String> {
    let kind = BurstKind::parse(kind).ok_or("unknown command")?;
    let login = channel_login.trim().trim_start_matches('#').to_lowercase();
    if login.is_empty() || channel_id.trim().is_empty() {
        return Err("Open a Twitch channel's chat first".into());
    }

    let (amount, text, delay_override) = match chat_burst::parse_args(kind, args)? {
        BurstArgs::Stop => {
            let message = match chat_burst::stop(&login) {
                Some(job) => chat_burst::stopped_message(&job),
                None => "Nothing is running here".to_string(),
            };
            return Ok(ChatBurstReply { message, status: None });
        }
        BurstArgs::Start { amount, text, delay_ms } => (amount, text, delay_ms),
    };

    let (delay_setting, vary) = settings;
    let delay_ms = delay_override.unwrap_or_else(|| chat_burst::clamp_delay(delay_setting));

    let lines = match kind {
        BurstKind::Spam => chat_burst::plan_spam(amount, &text, vary)?,
        BurstKind::Pyramid => chat_burst::plan_pyramid(amount, &text, vary)?,
    };

    // The signed-in account, the same identity a typed message goes out as.
    let sender_id = match AccountStore::primary() {
        Some(account) => account.user_id,
        None => TwitchService::get_user_info()
            .await
            .map_err(|e| format!("Sign in to Twitch first ({e})"))?
            .id,
    };

    let job = Job::new(&login, kind, lines.len(), delay_ms);
    chat_burst::claim(&job)?;
    let reply = ChatBurstReply {
        message: chat_burst::started_message(&job),
        status: Some(job.status("running", None)),
    };
    emit(&job.status("running", None));

    let broadcaster_id = channel_id.trim().to_string();
    tauri::async_runtime::spawn(async move {
        let send = |line: String| {
            let login = login.clone();
            let broadcaster_id = broadcaster_id.clone();
            let sender_id = sender_id.clone();
            async move {
                // Helix only, as the signed-in account. Twitch answers every
                // line (sent, dropped, or HTTP 429 for too fast), so the stop
                // rules see each refusal. No IRC fallback: IRC reports nothing
                // back and sends as whoever owns the connection. The line
                // echoes into chat like anything typed.
                let result = TwitchService::send_chat_message_helix(&broadcaster_id, &sender_id, &line, None)
                    .await
                    .map(|(_, is_sent, drop_reason)| (is_sent, drop_reason))
                    .map_err(|e| {
                        let status = e.downcast_ref::<HelixSendRefused>().map(|r| r.status);
                        (status, e.to_string())
                    });
                if matches!(result, Ok((true, _))) {
                    crate::services::chat_send_limiter::record_sent(&login, &line);
                }
                chat_burst::line_outcome(result)
            }
        };
        let end = chat_burst::run(job.clone(), lines, send, emit).await;
        chat_burst::release(&job);
        if announce_end {
            crate::services::slash_dispatch::post_room_notice(&end.channel, &end.label()).await;
        }
        log::info!(
            "[ChatBurst] {:?} in #{} ended {} at {}/{}{}",
            end.kind,
            end.channel,
            end.state,
            end.sent,
            end.total,
            end.reason.as_deref().map(|r| format!(": {r}")).unwrap_or_default()
        );
    });
    Ok(reply)
}

/// Stops the run going in a channel (the Stop button beside its progress).
#[tauri::command]
pub async fn stop_chat_burst(channel_login: String) -> Result<bool, String> {
    Ok(chat_burst::stop(channel_login.trim().trim_start_matches('#')).is_some())
}

/// The run going in a channel, for a chat box that opens while one runs.
#[tauri::command]
pub async fn get_chat_burst(channel_login: String) -> Result<Option<BurstStatus>, String> {
    Ok(chat_burst::current(channel_login.trim().trim_start_matches('#'))
        .map(|job| job.status("running", None)))
}

fn emit(status: &BurstStatus) {
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit(chat_burst::EVENT, status);
    }
}
