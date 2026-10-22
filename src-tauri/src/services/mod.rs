pub mod default_name_color;
pub mod account_roster;
pub mod watch_minutes;
pub mod link_detect;
pub mod background_service;
pub mod badge_feed;
pub mod badge_polling_service;
pub mod badge_earn;
pub mod badge_service;
pub mod browse_languages;
pub mod badge_standing;
pub mod badge_wall;
pub mod badge_window;
pub mod bttv_pro_service;

pub mod account_store;
pub mod activity_history_service;
// Phones only: ad-free live playback. Desktop keeps the ad-neutral core and
// its `playback.resolve` plugin seam; a phone can't host a plugin (it is a
// spawned native process), so the same work happens in-core there.
#[cfg(mobile)]
pub mod ad_bypass;
pub mod ad_detect;
pub mod app_paths;
#[cfg(target_os = "ios")]
pub mod ios_sheet;
pub mod auth_proxy;
pub mod cache_service;
pub mod cast_pick;
#[cfg(target_os = "android")]
pub mod cast_session;
pub mod changelog;
pub mod channel_media;
pub mod channel_avatars;
pub mod ll_diagnostics;
pub mod channel_points_websocket_service;
pub mod chat_logger_service;
pub mod automod_queue;
pub mod chat_bridge_route;
pub mod chat_history;
pub mod chat_rules;
pub mod chat_row_view;
pub mod shared_chat_emotes;
pub mod history_cosmetics;
pub mod command_catalog;
pub mod chat_send_limiter;
pub mod recent_emotes;
pub mod image_upload;
pub mod image_upload_store;
pub mod image_probe;
pub mod room_modes;
pub mod chatter_name;
pub mod text_spans;
pub mod chat_burst;
pub mod chat_service;
pub mod client_config;
pub mod client_identity;
pub mod version_report;
pub mod twitch_recap;
pub mod pronouns;
pub mod mention_ping;
pub mod mention_watch;
pub mod mention_match;
pub mod mention_inbox;
pub mod stream_manager;
pub mod streamer_mode;
pub mod suspicious_users;
pub mod user_notes;
pub mod mod_history;
pub mod chat_warning;
pub mod chatter_card;
pub mod touch_mod_slider;
pub mod room_mod_log;
pub mod unban_requests;
pub mod diagnostic_logger;
pub mod hls_projection;
pub mod hls_seq_ids;
#[cfg(windows)]
pub mod webview2_failures;
// Desktop-only: Discord Rich Presence (IPC to a running Discord client).
#[cfg(desktop)]
pub mod discord_service;
// macOS supplies its own socket discovery; see the file header for why the
// crate's Unix finder cannot locate Discord there.
#[cfg(target_os = "macos")]
pub mod discord_ipc_macos;
pub mod drop_playback;
pub mod drops_auth_service;
pub mod drop_words;
pub mod drops_overview;
pub mod drops_service;
pub mod evolution;
pub mod game_suggest;
pub mod wallet_glyph;
pub mod watched_chest;
pub mod reward_drops;
pub mod reward_redeem;
pub mod rewards_drops;
pub mod watch_drop;
pub mod emoji_service;
pub mod spellcheck;
pub mod asset_cache_queue;
pub mod emote_credit;
pub mod emote_match;
pub mod emote_picker;
pub mod emote_prefetch_service;
#[cfg(test)]
mod emote_golden;
pub mod emote_restamp;
pub mod emote_service;
pub mod emote_set_cache;
pub mod eventsub_budget;
pub mod eventsub_moderation;
pub mod eventsub_service;
pub mod ffz_room_badges;
pub mod file_log;
pub mod app_exit;
pub mod http;
pub mod hype_train_watch;
pub mod irc_service;
pub mod ivr;
pub mod late_third_party;
pub mod irc_transport;
pub mod kick_auth_service;
pub mod layout_service;
pub mod linked_accounts;
pub mod member_alias;
pub mod modroom_auth_service;
pub mod tiktok_auth_service;
pub mod youtube_auth_service;
pub mod favorite_categories;
pub mod hidden_channels;
pub mod favorite_live_service;
// The live line for the desktop chat dock and the phone chats.
pub mod chat_dock_live;
pub mod live_announce;
pub mod live_bookmark;
pub mod live_notification_service;
pub mod offline_room;
pub mod onsite_notifications;
pub mod membership_gifts;
pub mod membership_status;
pub mod provider_categories;
pub mod provider_live_service;
pub mod link_adapt;
pub mod ll_origin;
pub mod theme_palette;
pub mod settings_backups;
pub mod install_marker;
pub mod settings_catalog;
pub mod alert_types;
// Phones only: go-live push registration. Its rules are tested on desktop.
#[cfg(any(mobile, test))]
pub mod push_alerts;
// Phones only: the background go-live check and its never-twice record. Its
// rules are tested on desktop.
#[cfg(any(mobile, test))]
pub mod live_poll;
pub mod client_capabilities;
#[cfg(test)]
mod ll_soak;
#[cfg(test)]
mod link_soak;
pub mod log_service;
pub mod resource_log;
pub mod runtime_watchdog;
pub mod sign_in_profile;
pub mod ui_hang_watchdog;
pub mod window_aspect;
pub mod media_glow;
pub mod mod_log_storage_service;
pub mod moderation;
// Desktop-only: MultiNook multi-stream tiling is not part of the phone app.
#[cfg(desktop)]
pub mod multi_nook_server;
pub mod multi_nook_meta;
#[cfg(desktop)]
pub mod multi_nook_raids;
pub mod profile_cache_service;
pub mod providers;
pub mod quality;
pub mod my_clips;
pub mod relay_probe;
pub mod reminder_service;
pub mod seventv_auth_service;
pub mod seventv_cosmetics_resolver;
pub mod seventv_eventapi;
pub mod song_id;
pub mod stream_server;
pub mod sub_standing;
pub mod follow_lookup;
pub mod streamnook_backend;
pub mod css_fill;
pub mod cosmetics_registry;
pub mod member_profile;
pub mod member_rewards;
pub mod member_signin;
pub mod member_stats;
pub mod member_theme;
pub mod profile_view;
pub mod flv_fmp4;
pub mod tiktok_relay;
pub mod ts_fmp4;
pub mod webm_fmp4;
pub mod youtube_dash;
pub mod youtube_potoken;
pub mod youtube_sabr;
pub mod twitch_auth_service;
pub mod twitch_web_gql;
pub mod twitch_integrity;
pub mod bits;
pub mod count_text;
pub mod twitch_resolver;
pub mod twitch_limits;
pub mod token_vault;
#[cfg(target_os = "android")]
pub mod tv_screen;
pub mod twitch_chat_history;
pub mod twitch_service;
pub mod twitch_signin;
pub mod twitch_profile_edit;
pub mod universal_cache_service;
pub mod update_watch;
pub mod user_message_history_service;
pub(crate) mod channel_ids;
pub mod channel_link_service;
pub mod channel_points_summary;
pub mod channel_relationship;
pub mod channel_rewards;
pub mod channel_state;
// The phone chat pane's open rooms (both phone shells).
pub mod chat_tabs;
pub mod chat_picker;
// Rebuilds a dead Twitch chat connection with its rooms (the phones' reload and watchdog).
pub mod chat_recovery;
// A native shell's outage row and gap backfill for Twitch chat.
pub mod chat_outage;
pub mod collaboration;
pub mod shared_chat;
pub mod link_preview_cache;
pub mod shared_chat_source;
// The Custom interface font's files on the phones.
pub mod custom_font;
pub mod share_link;
pub mod slash_commands;
pub mod chat_top_stack;
pub mod poll_view;
pub mod predictions;
pub mod sleep_timer;
pub mod slash_dispatch;
pub mod nuke;
pub mod hls_kind;
pub mod home_snapshot;
pub mod unified_discover;
pub mod unified_following;
pub mod muted_segments;
pub mod vod_chapters;
pub mod vod_progress_service;
pub mod vod_replay;
pub mod vod_sync;
pub mod vod_storyboard;
pub mod watch_heartbeat_service;
pub mod whisper_history_service;
pub mod watch_session;
pub mod watch_start;
pub mod app_presence;
pub mod earning_gate;
pub mod whisper_inbox;
pub mod whisper_service;
pub mod whisper_storage_service;
pub mod window_visibility;
// The operating system's own notifications (desktop).
#[cfg(desktop)]
pub mod system_notify;
// Windows: the NSIS install, its updates, and the move from the portable exe.
#[cfg(windows)]
pub mod installer_update;
// A notification when someone mentions you in a Twitch chat (desktop).
#[cfg(desktop)]
pub mod mention_notify;
// Channel points and prediction notifications, drawn per channel (desktop).
#[cfg(desktop)]
pub mod points_notify;
