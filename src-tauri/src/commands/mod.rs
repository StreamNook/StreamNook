pub mod accounts;
pub mod announcements;
pub mod app;
pub mod automation;
pub mod badge_metadata;
pub mod badge_service;
pub mod badges;

pub mod cache;
pub mod channel_panels;
pub mod channel_state;
pub mod chat;
pub mod chat_identity;
pub mod chat_query;
pub mod moderation_tools;
pub mod components;
pub mod cosmetics_cache;
pub mod diagnostic_logging;
// Desktop-only feature commands, excluded from the phone app (watch/earn/chat only).
#[cfg(desktop)]
pub mod discord;
pub mod drops;
pub mod emoji;
pub mod emote_prefetch;
pub mod emotes;
pub mod eventsub;
pub mod ffz;
pub mod gifs;
pub mod home_snapshot;
pub mod helix;
pub mod hype_train;
pub mod identity;
pub mod justlog;
pub mod layout;
pub mod link_preview;
pub mod logs;
pub mod mod_log_storage;
pub mod modroom;
#[cfg(desktop)]
pub mod multi_nook;
pub mod plugins;
pub mod profile_cache;
pub mod provider_browse;
pub mod resub;
#[cfg(desktop)]
pub mod screen_capture;
pub mod session;
pub mod settings;
pub mod seventv;
pub mod song_id;
// Cosmetics (7TV paints/badges apply + auth status) are kept on mobile; only the
// individual 7TV login *popup* functions inside are #[cfg(desktop)]-gated.
pub mod seventv_cosmetics;
pub mod seventv_cosmetics_fetch;
pub mod streamnook_api;
pub mod streaming;
pub mod subscriptions;
pub mod twitch;
pub mod universal_cache;
pub mod user_profile;
pub mod vod_progress;
pub mod watch_streak;
pub mod whisper_storage;
