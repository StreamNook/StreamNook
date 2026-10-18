//! The slash dispatcher's way into the app (`services::slash_dispatch::Backend`):
//! each command it names runs the same Rust the page used to invoke by that
//! name, with the same arguments.

use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::Manager;

use crate::commands::emotes::EmoteServiceState;
use crate::commands::eventsub::EventSubServiceState;
use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::chat_history::{ChatHistory, SweepEntry};
use crate::services::chat_send_limiter::Role;
use crate::services::moderation::{ModAction, ModRequest};
use crate::services::slash_commands as sc;
use crate::services::slash_dispatch::{self, Backend, BoxFut, Me};
use crate::services::twitch_service::TwitchService;

pub struct TauriBackend {
    pub app: AppHandle,
    /// The window the command was typed in (`/disconnect` releases its chat).
    pub window: String,
}

/// A backend on the app's own handle, for work that starts outside a window
/// (a reminder firing).
pub fn global() -> Option<Arc<dyn Backend>> {
    let app = crate::services::providers::app_handle()?;
    Some(Arc::new(TauriBackend { app, window: "main".to_string() }))
}

fn s(a: &Value, k: &str) -> String {
    a[k].as_str().unwrap_or_default().to_string()
}

fn os(a: &Value, k: &str) -> Option<String> {
    a[k].as_str().map(str::to_string)
}

fn ou32(a: &Value, k: &str) -> Option<u32> {
    a[k].as_i64().map(|n| u32::try_from(n.max(0)).unwrap_or(u32::MAX))
}

fn to<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

fn unit(r: Result<(), String>) -> Result<Value, String> {
    r.map(|_| Value::Null)
}

/// One Twitch moderation action through `services::moderation`, from the
/// arguments the page's `ban_user` / `unban_user` / `delete_chat_message` took.
/// Answers the outcome (`message` is ready for a toast).
async fn moderate(a: &Value, action: ModAction) -> Result<Value, String> {
    let request = ModRequest {
        provider: None,
        broadcaster_id: os(a, "broadcasterId"),
        channel: None,
        target_user_id: os(a, "targetUserId"),
        target_name: os(a, "targetName"),
        action,
        reason: os(a, "reason"),
        use_saved_reason: false,
        typed: None,
    };
    to(crate::services::moderation::moderate(request).await.map_err(|e| e.to_string()))
}

impl TauriBackend {
    fn state(&self) -> tauri::State<'_, AppState> {
        self.app.state::<AppState>()
    }

    /// A scripted line with a reply parent or a pin: the same wait for a slot
    /// as `chat_send_paced`, then the send, then the pin.
    async fn paced_send_with(&self, a: &Value) -> Result<Value, String> {
        let login = s(a, "channel").trim().trim_start_matches('#').to_lowercase();
        let text = s(a, "text");
        let reply = os(a, "replyTo");
        let pin = a["pinAfterSend"].as_bool().unwrap_or(false);
        let sender = match crate::services::account_store::AccountStore::primary() {
            Some(acc) => acc.user_id,
            None => TwitchService::get_user_info()
                .await
                .map_err(|_| "Sign in to Twitch to send messages".to_string())?
                .id,
        };
        let broadcaster = match os(a, "channelId").filter(|id| !id.is_empty()) {
            Some(id) => id,
            None => sc::user_by_login(&login)
                .await?
                .and_then(|u| u["id"].as_str().map(str::to_string))
                .ok_or_else(|| format!("There is no Twitch channel named {login}"))?,
        };
        crate::services::chat_send_limiter::acquire(&login, &text).await.map_err(|r| r.text())?;
        let sent = crate::services::chat_service::ChatService::send_message(
            &text,
            reply.as_deref(),
            Some(&login),
            Some(&broadcaster),
            Some(&sender),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
        if pin && sent.is_sent {
            if let Some(id) = sent.message_id.as_deref() {
                crate::commands::twitch::pin_chat_message(broadcaster.clone(), id.to_string(), None).await?;
            }
        }
        Ok(json!({ "is_sent": sent.is_sent, "drop_reason": sent.drop_reason }))
    }

    async fn emote_set(&self, channel: &str, id: Option<String>, fresh: bool) -> Result<crate::services::emote_service::EmoteSet, String> {
        let state = self.app.try_state::<EmoteServiceState>().ok_or("emotes are not ready")?;
        let token = TwitchService::get_token().await.ok();
        let service = state.0.read().await;
        if fresh {
            if let Some(id) = id.as_deref() {
                service.invalidate_channel(id).await;
            }
        }
        service.fetch_channel_emotes(Some(channel.to_string()), id, token, None).await.map_err(|e| e.to_string())
    }

    async fn run(&self, cmd: &str, a: Value) -> Result<Value, String> {
        use crate::commands::{settings as st, slash_commands as sl, twitch as tw};
        let app = self.app.clone();
        match cmd {
            "get_user_by_login" => to(tw::get_user_by_login(s(&a, "login")).await),
            // Bans, timeouts, unbans and deletes go through the one moderation
            // door every surface uses (`services::moderation`).
            "ban_user" => {
                let action = match ou32(&a, "duration") {
                    Some(seconds) => ModAction::Timeout { seconds },
                    None => ModAction::Ban,
                };
                moderate(&a, action).await
            }
            "unban_user" => moderate(&a, ModAction::Unban { timeout: false }).await,
            "clear_chat" => unit(tw::clear_chat(s(&a, "broadcasterId")).await),
            "delete_chat_message" => moderate(&a, ModAction::Delete { message_id: s(&a, "messageId") }).await,
            "update_chat_settings" => unit(tw::update_chat_settings(s(&a, "broadcasterId"), a["settings"].clone()).await),
            "send_chat_announcement" => {
                unit(tw::send_chat_announcement(s(&a, "broadcasterId"), s(&a, "message"), os(&a, "color")).await)
            }
            "send_shoutout" => unit(tw::send_shoutout(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "start_commercial" => {
                let length = a["length"].as_i64().and_then(|n| u32::try_from(n).ok()).ok_or("Commercial length must be 30 to 180 seconds")?;
                unit(tw::start_commercial(s(&a, "broadcasterId"), length).await)
            }
            "add_channel_moderator" => unit(tw::add_channel_moderator(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "remove_channel_moderator" => unit(tw::remove_channel_moderator(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "add_channel_vip" => unit(tw::add_channel_vip(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "remove_channel_vip" => unit(tw::remove_channel_vip(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "start_raid" => unit(tw::start_raid(s(&a, "broadcasterId"), s(&a, "targetUserId")).await),
            "cancel_raid" => unit(tw::cancel_raid(s(&a, "broadcasterId")).await),
            "create_stream_marker" => unit(tw::create_stream_marker(s(&a, "userId"), os(&a, "description")).await),
            "update_user_chat_color" => unit(tw::update_user_chat_color(s(&a, "targetUserId"), s(&a, "color")).await),
            "block_user" => unit(tw::block_user(s(&a, "targetUserId")).await),
            "unblock_user" => unit(tw::unblock_user(s(&a, "targetUserId")).await),
            "send_whisper" => unit(tw::send_whisper(s(&a, "toUserId"), s(&a, "message")).await),
            "stop_chat" => to(crate::services::chat_service::ChatService::stop(&self.window).await.map_err(|e| e.to_string())),
            "get_chatters_by_role" => tw::get_chatters_by_role(self.state(), s(&a, "channelLogin")).await,
            "slash_role_list" => to(sl::slash_role_list(self.state(), s(&a, "channel"), s(&a, "role")).await),
            "get_channel_chatters" => tw::get_channel_chatters(self.state(), s(&a, "broadcasterId"), s(&a, "channelLogin")).await,
            "update_suspicious_user_status" => {
                unit(tw::update_suspicious_user_status(s(&a, "broadcasterId"), s(&a, "targetUserId"), s(&a, "status")).await)
            }
            "warn_chat_user" => unit(tw::warn_chat_user(s(&a, "broadcasterId"), s(&a, "targetUserId"), s(&a, "reason")).await),
            "update_shield_mode" => {
                unit(tw::update_shield_mode(s(&a, "broadcasterId"), a["isActive"].as_bool().unwrap_or(false)).await)
            }
            "update_channel_info" => unit(
                crate::commands::moderation_tools::update_channel_info(s(&a, "broadcasterId"), os(&a, "title"), os(&a, "gameName")).await,
            ),
            "get_active_poll" => to(tw::get_active_poll(s(&a, "broadcasterId")).await),
            "end_poll" => tw::end_poll(s(&a, "broadcasterId"), s(&a, "pollId"), s(&a, "status")).await,
            "finish_prediction" => {
                let action = serde_json::from_value(a["action"].clone()).map_err(|e| e.to_string())?;
                to(tw::finish_prediction(s(&a, "broadcasterId"), action, os(&a, "outcome")).await)
            }
            "add_blocked_term" => tw::add_blocked_term(s(&a, "broadcasterId"), s(&a, "text")).await,
            "remove_blocked_term" => unit(tw::remove_blocked_term(s(&a, "broadcasterId"), s(&a, "text")).await),
            "pin_chat_message" => {
                unit(tw::pin_chat_message(s(&a, "broadcasterId"), s(&a, "messageId"), ou32(&a, "durationSeconds")).await)
            }
            "unpin_chat_message" => unit(tw::unpin_chat_message(s(&a, "broadcasterId"), None).await),
            "open_url" => unit(tw::open_browser_url(app, s(&a, "url")).await),
            "copy_text" => unit(sl::slash_copy_text(app, s(&a, "text"))),
            "open_subscribe_window" => {
                #[cfg(desktop)]
                let opened = tw::open_subscribe_window(app, s(&a, "channelLogin"), os(&a, "title"), os(&a, "provider"));
                #[cfg(mobile)]
                let opened = tw::open_subscribe_window(app, s(&a, "channelLogin"), os(&a, "title"), os(&a, "provider")).await;
                to(opened)
            }
            "set_chat_user_hidden" => unit(
                st::set_chat_user_hidden(app, s(&a, "name"), os(&a, "channelKey"), a["hidden"].as_bool().unwrap_or(false), self.state())
                    .await,
            ),
            "set_chat_user_override" => unit(
                st::set_chat_user_override(app, s(&a, "userId"), os(&a, "username"), os(&a, "field"), os(&a, "value"), self.state())
                    .await,
            ),
            "patch_settings" => {
                let patch = a["patch"].as_object().cloned().unwrap_or_default();
                unit(st::patch_settings(app, patch, None, self.state()).await)
            }
            "get_default_settings" => to(Ok(st::get_default_settings())),
            "get_app_version" => Ok(json!(crate::commands::app::get_app_version())),
            "get_system_info" => Ok(json!(crate::commands::app::get_system_info())),
            "get_settings_dir" => to(st::get_settings_dir().await),
            "get_chat_log_dir" => to(crate::commands::chat::get_chat_log_dir(self.state()).await),
            "get_chat_lifecycle_log" => to(crate::commands::chat::get_chat_lifecycle_log().await),
            "is_eventsub_connected" => match self.app.try_state::<EventSubServiceState>() {
                Some(state) => to(crate::commands::eventsub::is_eventsub_connected(state).await),
                None => Ok(json!(false)),
            },
            "get_eventsub_session_id" => match self.app.try_state::<EventSubServiceState>() {
                Some(state) => to(crate::commands::eventsub::get_eventsub_session_id(state).await),
                None => Ok(Value::Null),
            },
            "open_logs_folder" => unit(crate::commands::logs::open_logs_folder().await),
            "chat_send_budget" => to(Ok(crate::services::chat_send_limiter::budget(&s(&a, "channel")))),
            "chat_send_paced" => {
                let channel = s(&a, "channel");
                if let Some((provider, room)) = channel.split_once(':') {
                    return to(crate::commands::chat::provider_send_message(provider.to_string(), room.to_string(), s(&a, "text"), os(&a, "replyTo")).await);
                }
                if a.get("replyTo").is_some() || a.get("pinAfterSend").is_some() {
                    return self.paced_send_with(&a).await;
                }
                to(crate::services::chat_send_limiter::chat_send_paced(channel, os(&a, "channelId"), s(&a, "text")).await)
            }
            "get_stream" => {
                let login = s(&a, "login");
                let v = sc::helix(reqwest::Method::GET, &format!("streams?user_login={login}"), None).await?;
                Ok(v["data"].as_array().and_then(|d| d.first()).cloned().unwrap_or(Value::Null))
            }
            "refresh_emotes" => {
                let set = self.emote_set(&s(&a, "channel"), os(&a, "channelId"), true).await?;
                Ok(json!(set.twitch.len() + set.bttv.len() + set.seven_tv.len() + set.ffz.len()))
            }
            "emote_counts" => match self.emote_set(&s(&a, "channel"), os(&a, "channelId"), false).await {
                Ok(set) => Ok(json!({ "twitch": set.twitch.len(), "bttv": set.bttv.len(), "7tv": set.seven_tv.len(), "ffz": set.ffz.len() })),
                Err(_) => Ok(Value::Null),
            },
            "seventv_graphql_authed" => {
                crate::commands::seventv_cosmetics::seventv_graphql_authed(
                    s(&a, "query"),
                    Some(a["variables"].clone()).filter(|v| !v.is_null()),
                    os(&a, "operationName"),
                    os(&a, "accountId"),
                )
                .await
            }
            "refresh_favorites" => unit(crate::commands::provider_browse::refresh_favorites(app, self.state(), os(&a, "key")).await),
            "run_chat_burst" => {
                let settings = self
                    .state()
                    .settings
                    .lock()
                    .map(|st| (st.chat_burst.delay_ms, st.chat_burst.vary_repeats))
                    .map_err(|_| "settings unavailable".to_string())?;
                to(crate::commands::chat_burst::start_burst(
                    &s(&a, "kind"),
                    &s(&a, "args"),
                    &s(&a, "channelLogin"),
                    &s(&a, "channelId"),
                    settings,
                    a["announceEnd"].as_bool().unwrap_or(false),
                )
                .await)
            }
            "post_reminder" => {
                let copies = crate::services::reminder_service::clamp_repeat(a["repeat"].as_f64());
                let posted =
                    crate::services::reminder_service::post_text(&s(&a, "channel"), os(&a, "channelId").as_deref(), &s(&a, "message"), copies).await;
                Ok(json!(posted))
            }
            "slash_vote" => to(sl::slash_vote(s(&a, "channelId"), s(&a, "choice")).await),
            "slash_chat_rules" => to(sl::slash_chat_rules(s(&a, "channel")).await),
            "slash_followage" => to(sl::slash_followage(s(&a, "channel"), os(&a, "user")).await),
            "slash_follower_count" => to(sl::slash_follower_count(s(&a, "channel")).await),
            "slash_account_age" => to(sl::slash_account_age(s(&a, "user")).await),
            "slash_viewer_count" => to(sl::slash_viewer_count(s(&a, "channel")).await),
            "slash_founders" => to(sl::slash_founders(s(&a, "channel")).await),
            "slash_name_history" => to(sl::slash_name_history(s(&a, "user")).await),
            "slash_recent_messages" => {
                to(sl::slash_recent_messages(s(&a, "channel"), s(&a, "login"), a["limit"].as_u64().map(|n| n as usize)).await)
            }
            "slash_last_message" => to(sl::slash_last_message(s(&a, "channel"), s(&a, "login")).await),
            "slash_delete_message" => to(sl::slash_delete_message(s(&a, "broadcasterId"), s(&a, "channel"), s(&a, "target")).await),
            "slash_automod" => to(sl::slash_automod(s(&a, "channel"), os(&a, "messageId"), a["allow"].as_bool().unwrap_or(false)).await),
            "slash_low_trust" => to(sl::slash_low_trust(s(&a, "channel")).await),
            "slash_shared_chat" => to(sl::slash_shared_chat(s(&a, "broadcasterId"), s(&a, "channel")).await),
            "slash_raid_candidates" => to(sl::slash_raid_candidates(self.state()).await),
            "slash_raid_now" => unit(sl::slash_raid_now(s(&a, "broadcasterId")).await),
            "slash_lead_mod" => {
                to(sl::slash_lead_mod(s(&a, "broadcasterId"), s(&a, "target"), a["assign"].as_bool().unwrap_or(false)).await)
            }
            "slash_editor" => to(sl::slash_editor(s(&a, "broadcasterId"), s(&a, "target"), a["add"].as_bool().unwrap_or(false)).await),
            "slash_mass_unban_start" => {
                let channel = s(&a, "channel");
                unit(
                    sc::start_mass_unban(s(&a, "broadcasterId"), move |p| {
                        let channel = channel.clone();
                        tauri::async_runtime::spawn(async move {
                            slash_dispatch::post_room_notice(&channel, &p.message).await;
                        });
                    })
                    .await,
                )
            }
            "slash_mass_unban_stop" => Ok(json!(sl::slash_mass_unban_stop(s(&a, "broadcasterId")))),
            "slash_add_stream_highlight" => to(sl::slash_add_stream_highlight(s(&a, "channel"), os(&a, "comment")).await),
            "slash_open_stream_highlights" => to(sl::slash_open_stream_highlights().await),
            "slash_open_streamlink" => to(sl::slash_open_streamlink(s(&a, "channel"), os(&a, "quality")).await),
            "slash_send_raw_irc" => unit(sl::slash_send_raw_irc(s(&a, "line")).await),
            "slash_inject_fake_message" => unit(sl::slash_inject_fake_message(s(&a, "line")).await),
            "slash_run_program" => to(sl::slash_run_program(app, s(&a, "commandLine")).await),
            "slash_export_text" => to(sl::slash_export_text(s(&a, "file"), s(&a, "text")).await),
            "slash_open_export" => to(sl::slash_open_export(os(&a, "file")).await),
            other => Err(format!("unknown command {other}")),
        }
    }
}

impl Backend for TauriBackend {
    fn invoke<'a>(&'a self, cmd: &'a str, args: Value) -> BoxFut<'a, Result<Value, String>> {
        Box::pin(self.run(cmd, args))
    }

    fn me(&self) -> Option<Me> {
        crate::services::account_store::AccountStore::primary().map(|a| Me { id: a.user_id, login: a.login, display_name: a.display_name })
    }

    fn role(&self, channel: &str) -> Role {
        crate::services::chat_send_limiter::budget(channel).role
    }

    fn settings(&self) -> Value {
        self.state().settings.lock().ok().and_then(|s| serde_json::to_value(&*s).ok()).unwrap_or(Value::Null)
    }

    fn last_whisper(&self) -> Option<(String, String, String)> {
        crate::services::whisper_inbox::last_from()
    }

    fn history(&self, channel: &str, since_ms: i64) -> Vec<SweepEntry> {
        ChatHistory::since(channel, since_ms)
    }

    fn latest_message_id(&self, channel: &str, login: &str) -> Option<String> {
        sc::last_message_of(channel, &login.to_lowercase()).map(|h| h.id).filter(|id| !id.starts_with("local-"))
    }

    fn is_open<'a>(&'a self, channel: &'a str) -> BoxFut<'a, bool> {
        Box::pin(async move {
            if channel.contains(':') {
                return true;
            }
            crate::services::irc_service::IrcService::is_joined(&channel.to_lowercase()).await
        })
    }

    fn post_notice<'a>(&'a self, channel: &'a str, text: String) -> BoxFut<'a, ()> {
        Box::pin(async move { slash_dispatch::post_room_notice(channel, &text).await })
    }

    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    fn new_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
