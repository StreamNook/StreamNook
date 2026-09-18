pub mod default_name_color;
pub mod link_detect;
pub mod background_service;
pub mod badge_feed;
pub mod badge_polling_service;
pub mod badge_service;
pub mod bttv_pro_service;

pub mod account_store;
// Android-only: ad-free live playback. Desktop keeps the ad-neutral core and
// its `playback.resolve` plugin seam; the phone can't host a plugin (it is a
// spawned native process), so the same work happens in-core there.
#[cfg(target_os = "android")]
pub mod ad_bypass;
pub mod ad_detect;
pub mod app_paths;
pub mod auth_proxy;
pub mod cache_service;
pub mod ll_diagnostics;
pub mod channel_points_websocket_service;
pub mod chat_logger_service;
pub mod automod_queue;
pub mod chat_history;
pub mod chat_rules;
pub mod chat_service;
pub mod pronouns;
pub mod streamer_mode;
pub mod suspicious_users;
pub mod user_notes;
pub mod cookie_jar_service;
pub mod diagnostic_logger;
pub mod hls_projection;
// Desktop-only: Discord Rich Presence (IPC to a running Discord client).
#[cfg(desktop)]
pub mod discord_service;
// macOS supplies its own socket discovery; see the file header for why the
// crate's Unix finder cannot locate Discord there.
#[cfg(target_os = "macos")]
pub mod discord_ipc_macos;
pub mod drops_auth_service;
pub mod drops_service;
pub mod emoji_service;
pub mod emote_prefetch_service;
pub mod emote_service;
pub mod emote_set_cache;
pub mod eventsub_moderation;
pub mod eventsub_service;
pub mod file_log;
pub mod http;
pub mod irc_service;
pub mod irc_transport;
pub mod kick_auth_service;
pub mod layout_service;
pub mod modroom_auth_service;
pub mod youtube_auth_service;
pub mod favorite_live_service;
pub mod live_notification_service;
pub mod provider_live_service;
pub mod ll_origin;
#[cfg(test)]
mod ll_soak;
pub mod log_service;
pub mod resource_log;
pub mod runtime_watchdog;
pub mod secure_store;
pub mod ui_hang_watchdog;
pub mod window_aspect;
pub mod media_glow;
pub mod mod_log_storage_service;
// Desktop-only: MultiNook multi-stream tiling is not part of the phone app.
#[cfg(desktop)]
pub mod multi_nook_server;
pub mod profile_cache_service;
pub mod providers;
pub mod quality;
pub mod seventv_auth_service;
pub mod seventv_eventapi;
pub mod song_id;
pub mod stream_server;
pub mod ts_fmp4;
pub mod webm_fmp4;
pub mod youtube_dash;
pub mod youtube_potoken;
pub mod youtube_sabr;
pub mod twitch_auth_service;
pub mod twitch_resolver;
pub mod twitch_limits;
pub mod twitch_service;
pub mod universal_cache_service;
pub mod user_message_history_service;
pub mod channel_state;
pub mod hls_kind;
pub mod home_snapshot;
pub mod muted_segments;
pub mod vod_progress_service;
pub mod watch_heartbeat_service;
pub mod whisper_history_service;
pub mod whisper_service;
pub mod whisper_storage_service;
pub mod window_visibility;
