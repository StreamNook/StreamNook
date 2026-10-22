pub mod accounts;
pub mod activity_history;
pub mod announcements;
pub mod app;
pub mod automation;
pub mod badge_metadata;
pub mod badge_service;
pub mod badges;

pub mod cache;
pub mod channel_links;
pub mod channel_panels;
pub mod channel_state;
pub mod chat;
pub mod chat_burst;
// The phone chat pane's tabs and its Add a chat sheet (registered on the phones only).
pub mod chat_tabs;
// The chats docked in the main window (desktop only).
#[cfg(desktop)]
pub mod chat_dock;
pub mod sounds;
pub mod chat_identity;
pub mod chat_query;
pub mod moderation_tools;
pub mod image_upload;
pub mod components;
pub mod cosmetics_cache;
pub mod diagnostic_logging;
// Desktop-only feature commands, excluded from the phone app (watch/earn/chat only).
#[cfg(desktop)]
pub mod discord;
pub mod drops;
pub mod emoji;
pub mod spellcheck;
pub mod watch_session;
pub mod emote_prefetch;
pub mod emotes;
pub mod eventsub;
pub mod ffz;
pub mod gifs;
pub mod bits;
pub mod home_snapshot;
pub mod shared_chat;
pub mod helix;
pub mod hype_train;
pub mod identity;
pub mod justlog;
pub mod layout;
pub mod link_preview;
pub mod fonts;
pub mod logs;
pub mod media_glow;
pub mod mod_log_storage;
pub mod modroom;
#[cfg(desktop)]
pub mod multi_nook;
// Notifications shown in MultiChat while the main window is off screen.
#[cfg(desktop)]
pub mod notification_mirror;
pub mod overlay_file;
pub mod plugins;
pub mod profile_cache;
pub mod provider_browse;
// Opening and placing the chat popouts (desktop windows only).
#[cfg(desktop)]
pub mod popout_window;
// The popped-out emote menu and the chat box it types into.
#[cfg(desktop)]
pub mod emote_palette;
pub mod resub;
#[cfg(desktop)]
pub mod screen_capture;
pub mod session;
pub mod slash_commands;
pub mod slash_backend;
pub mod settings;
pub mod settings_fields;
pub mod settings_lists;
pub mod settings_extras;
pub mod seventv;
pub mod song_id;
// Cosmetics (7TV paints/badges apply + auth status) are kept on mobile; only the
// individual 7TV login *popup* functions inside are #[cfg(desktop)]-gated.
pub mod seventv_cosmetics;
pub mod sn_cosmetics;
pub mod seventv_cosmetics_fetch;
pub mod streamnook_api;
pub mod stream_manager;
pub mod streaming;
pub mod subscriptions;
pub mod text_segments;
pub mod twitch;
pub mod universal_cache;
pub mod user_profile;
pub mod vod_progress;
pub mod watch_streak;
pub mod whisper_storage;
pub mod mention_inbox;
pub mod window_state;
