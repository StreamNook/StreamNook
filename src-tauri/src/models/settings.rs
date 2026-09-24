use crate::services::background_service::BackgroundService;
use crate::services::drops_service::DropsService;
use crate::services::emote_service::EmoteService;
use crate::services::layout_service::LayoutService;
use crate::services::twitch_auth_service::TwitchAuthService;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{Mutex as TokioMutex, RwLock};

/// Web Audio processing for the player: a dynamics compressor followed by a
/// makeup-gain stage. Together they level out loud/quiet swings and let the
/// stream be pushed louder than the source without the clipping you'd get from
/// raising volume past 100%. Off by default; flipping `enabled` applies the
/// values below, which are tuned as a gentle, pleasant starting point the user
/// can then adjust.
#[derive(Serialize, Deserialize, Clone)]
pub struct AudioBoostSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_audio_gain")]
    pub gain: f32, // Makeup gain multiplier applied after compression (1.0 = unity)
    #[serde(default = "default_audio_threshold")]
    pub threshold: f32, // dB, level where compression begins
    #[serde(default = "default_audio_knee")]
    pub knee: f32, // dB, how gradually compression ramps in around the threshold
    #[serde(default = "default_audio_ratio")]
    pub ratio: f32, // x:1 compression ratio above the threshold
    #[serde(default = "default_audio_attack")]
    pub attack: f32, // seconds to clamp down once over the threshold
    #[serde(default = "default_audio_release")]
    pub release: f32, // seconds to ease back off once under the threshold
}

fn default_audio_gain() -> f32 {
    1.5
}
fn default_audio_threshold() -> f32 {
    -30.0
}
fn default_audio_knee() -> f32 {
    30.0
}
fn default_audio_ratio() -> f32 {
    6.0
}
fn default_audio_attack() -> f32 {
    0.003
}
fn default_audio_release() -> f32 {
    0.25
}

impl Default for AudioBoostSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            gain: default_audio_gain(),
            threshold: default_audio_threshold(),
            knee: default_audio_knee(),
            ratio: default_audio_ratio(),
            attack: default_audio_attack(),
            release: default_audio_release(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct VideoPlayerSettings {
    pub max_buffer_length: u32,
    pub autoplay: bool,
    pub muted: bool,
    pub volume: f32,
    pub start_quality: i32,
    pub lock_aspect_ratio: bool,
    /// When true the letterbox bars stay solid black (cinema). Default false makes
    /// them match the theme background so the video appears to float.
    #[serde(default)]
    pub cinema_mode: bool,
    #[serde(default)]
    pub audio_boost: AudioBoostSettings,
    /// Drive playback through the parts-based LL-HLS origin (Twitch-parity
    /// latency) instead of the whole-segment path. On by default since 2026-09-21,
    /// when it was measured level with twitch.tv on H.264 and 1440p channels; the
    /// frontend syncs it to the runtime kill switch at startup. Off remains the
    /// fallback for a channel or machine that stutters on it.
    #[serde(default = "default_true")]
    pub experimental_low_latency: bool,
    /// Set once the engine has been switched on by default for this install.
    /// Every file written before the default flipped carries the old `false`
    /// that nobody chose; `enable_low_latency_engine_once` flips it exactly one
    /// time, and a viewer who turns it off afterwards stays off.
    #[serde(default)]
    pub low_latency_engine_defaulted: bool,
    /// Displayed "behind live" the viewer wants to ride at on the low-latency path
    /// (seconds). Lower rides closer to live but needs a capable system/connection;
    /// higher is safer. The player adds the display calibration to get the real cushion
    /// and governor target. Default 2.5.
    /// The viewer's live-edge gap in displayed seconds, or `None` for the
    /// automatic per-path default the frontend resolves (parts origin,
    /// promoted low-latency broadcast, normal-latency broadcast). Was a
    /// plain `f32` defaulting to 6.0; see `retire_legacy_live_edge_gap`.
    #[serde(default)]
    pub ll_target_latency: Option<f32>,
    /// Scrolling over the player adjusts volume. On by default.
    #[serde(default = "default_true")]
    pub scroll_volume: bool,
    /// Scrolling down over the player opens the channel About drawer. On by
    /// default. When `scroll_volume` is also on, the plain wheel belongs to
    /// volume and this moves to Shift + scroll down, so both can coexist.
    #[serde(default = "default_true")]
    pub scroll_about_reveal: bool,
    /// Middle-clicking the player toggles mute. On by default.
    #[serde(default = "default_true")]
    pub middle_click_mute: bool,
    /// How much one wheel notch moves the volume (0.01-0.25). Default 5%.
    #[serde(default = "default_wheel_volume_step")]
    pub wheel_volume_step: f32,
    /// Reopen a VOD where the viewer left off. On by default; off starts every
    /// VOD from the top (positions are still recorded for the cards).
    #[serde(default = "default_true")]
    pub resume_vod_playback: bool,
    /// Ad-free live playback (Android). Routes the playlist through a public
    /// relay and strips ad segments out of what the player is served. On by
    /// default, matching the behavior the phone app shipped with through 7.8.6.
    /// Read only by the Android build; desktop resolves through its plugin
    /// seam and ignores this.
    #[serde(default = "default_true")]
    pub ad_bypass_enabled: bool,
    /// Relay bases to prefer, comma or newline separated. Empty means the
    /// bundled pool, which is what almost everyone should use.
    #[serde(default)]
    pub ad_bypass_proxies: String,
    /// What leaving the app does while a stream is playing (Android only).
    ///
    /// "pip"   - float the video in a system picture-in-picture window. Default,
    ///           and the behaviour the phone app has always had.
    /// "audio" - no floating window: the stream drops to its audio-only
    ///           rendition and keeps playing behind a media notification, which
    ///           taps back into the app.
    ///
    /// The audio path is not merely a preference. Backgrounding without PiP
    /// destroys the activity's window surface, and Chromium tears down a media
    /// pipeline that has a video track and nowhere to render it. Audio-only has
    /// nothing to render, so it survives.
    #[serde(default = "default_background_mode")]
    pub background_mode: String,
}

fn default_background_mode() -> String {
    "pip".to_string()
}

fn default_wheel_volume_step() -> f32 {
    0.05
}

impl Default for VideoPlayerSettings {
    fn default() -> Self {
        Self {
            max_buffer_length: 120,
            autoplay: true,
            muted: false,
            volume: 1.0,
            start_quality: -1,
            lock_aspect_ratio: true,
            cinema_mode: false,
            audio_boost: AudioBoostSettings::default(),
            experimental_low_latency: true,
            low_latency_engine_defaulted: true,
            ll_target_latency: None,
            ad_bypass_enabled: true,
            ad_bypass_proxies: String::new(),
            background_mode: default_background_mode(),
            scroll_volume: true,
            scroll_about_reveal: true,
            middle_click_mute: true,
            wheel_volume_step: 0.05,
            resume_vod_playback: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CacheSettings {
    pub enabled: bool,
    pub expiry_days: u32,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            expiry_days: 7,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StreamlinkSettings {
    /// Total budget (seconds) for the native resolver's retry-until-live loop.
    pub stream_timeout: u32,
    /// Delay (seconds) between native resolve attempts (0 = single attempt).
    pub retry_streams: u32,
    /// Request Twitch's Enhanced Broadcasting variants (h265 + AV1 in addition
    /// to h264) when resolving.
    #[serde(default = "default_true")]
    pub enhanced_codecs: bool,
}

impl Default for StreamlinkSettings {
    fn default() -> Self {
        Self {
            stream_timeout: 60,
            retry_streams: 3,
            enhanced_codecs: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatDesignSettings {
    pub show_dividers: bool,
    pub alternating_backgrounds: bool,
    pub message_spacing: u32,    // 0-20 pixels
    pub font_size: u32,          // 10-20 pixels
    pub font_weight: u32,        // 300-700
    pub mention_color: String,   // Hex color for @ mentions
    pub reply_color: String,     // Hex color for reply threads
    pub mention_animation: bool, // Enable red-shift animation for mentions
    #[serde(default)]
    pub show_timestamps: bool, // Show timestamp next to each message
    #[serde(default)]
    pub show_timestamp_seconds: bool, // Include seconds in timestamps
    /// "12h" (default) | "24h". Read by irc_service's timestamp formatter.
    #[serde(default = "default_timestamp_format")]
    pub timestamp_format: String,
    // The fields below were added to the TS type over time but were missing here,
    // so they silently failed to persist (serde drops unknown fields on save).
    // Each carries a serde default matching the frontend default so old
    // settings.json files (which lack the field) still load.
    #[serde(default = "default_emote_scale")]
    pub emote_scale: f64, // Inline emote size multiplier (0.5-3)
    /// "always" (default) | "hover" | "never": animated emotes play, play only
    /// while the row is hovered, or show their first frame. A real CPU lever.
    #[serde(default = "default_animate_emotes")]
    pub animate_emotes: String,
    /// Twitch chat GIFs (Tier 2/3 subscribers): render the asset, or a chip
    /// that reveals it on click. Default true.
    #[serde(default = "default_true")]
    pub show_chat_gifs: bool,
    /// Opacity of backfilled history rows, 0-100 (100 = same as live).
    #[serde(default = "default_backfill_opacity")]
    pub backfill_opacity: u32,
    #[serde(default = "default_emote_margin")]
    pub emote_margin: f64, // Horizontal margin around emotes, rem
    #[serde(default = "default_emote_hover_size")]
    pub emote_hover_size: u32, // Enlarged emote height in hover preview, px
    #[serde(default = "default_deleted_message_style")]
    pub deleted_message_style: String, // strikethrough | hidden | dimmed | keep
    #[serde(default)]
    pub hide_shared_chat: bool,
    #[serde(default = "default_true")]
    pub paint_mentions_in_body: bool,
    #[serde(default)]
    pub compact_emote_tooltips: bool,
    #[serde(default = "default_true")]
    pub ffz_emote_effects: bool,
    #[serde(default = "default_true")]
    pub bttv_emote_modifiers: bool,
    /// Render the last emote of a "Gigantify an Emote" power-up message
    /// (msg-id gigantified-emote-message) at 4x below the message body.
    #[serde(default = "default_true")]
    pub giant_emotes: bool,
    /// Which half of the user card opens first: the profile body (default) or
    /// their recent messages.
    #[serde(default)]
    pub user_card_opens_messages: bool,
    #[serde(default = "default_true")]
    pub seventv_emote_notices: bool,
    #[serde(default = "default_true")]
    pub link_previews: bool,
    #[serde(default)]
    pub link_preview_keep_link: bool,
    #[serde(default = "default_true")]
    pub shorten_links: bool,
    #[serde(default)]
    pub link_preview_trusted_domains: Vec<String>,
    // Username prefix styling: separator glyph + name emphasis + color source.
    #[serde(default = "default_username_separator")]
    pub username_separator: String, // none | colon | dot | arrow | pipe | dash
    #[serde(default = "default_username_style")]
    pub username_style: String, // plain | bar | chip | brackets | dot
    #[serde(default = "default_username_accent_source")]
    pub username_accent_source: String, // user | theme
    #[serde(default = "default_true")]
    pub drag_moderation_enabled: bool, // deprecated: superseded by mod_action_style
    #[serde(default = "default_mod_action_style")]
    pub mod_action_style: String, // buttons | drag | both
    #[serde(default = "default_mod_drag_layout")]
    pub mod_drag_layout: String, // column | bar
    #[serde(default = "default_pinned_collapsed_style")]
    pub pinned_collapsed_style: String, // bar | hidden
    #[serde(default = "default_mod_pin_style")]
    pub mod_pin_style: String, // inline | drag | both
    /// "hsl_loop" (default) walks a chatter's color toward the readable side
    /// of the theme; "off" renders it as Twitch sent it.
    #[serde(default = "default_name_color_adjustment")]
    pub name_color_adjustment: String,
    // Badges, entrance, emoji, emote extras, replies and links: the render
    // options the OBS overlay had that chat did not.
    #[serde(default = "default_true")]
    pub show_badges: bool,
    #[serde(default = "default_badge_scale")]
    pub badge_scale: f64,
    #[serde(default = "default_true")]
    pub show_third_party_badges: bool,
    #[serde(default)]
    pub hidden_badge_providers: Vec<String>,
    #[serde(default = "default_message_entrance")]
    pub message_entrance: String, // none | fade | slide | rise
    #[serde(default = "default_emoji_style")]
    pub emoji_style: String, // system | apple | google | twitter | facebook
    #[serde(default = "default_true")]
    pub show_personal_emotes: bool,
    #[serde(default = "default_giant_emote_align")]
    pub giant_emote_align: String, // left | center | right | inline
    #[serde(default = "default_true")]
    pub show_avatars: bool,
    #[serde(default)]
    pub show_at_sign: bool,
    #[serde(default = "default_reply_style")]
    pub reply_style: String, // full | mention | off
    #[serde(default)]
    pub link_color: String,
    #[serde(default = "default_true")]
    pub link_underline: bool,
    // Optional on the TS side and left optional here on purpose: the frontend
    // applies its own default when the key is absent, and turning an absent
    // key into a concrete value on save would change what a fresh install
    // sees. Absent stays absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_font_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_colon: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_start_collapsed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polls_start_collapsed: Option<bool>,
}

fn default_name_color_adjustment() -> String {
    "hsl_loop".to_string()
}

fn default_badge_scale() -> f64 {
    1.0
}

fn default_message_entrance() -> String {
    "none".to_string()
}

fn default_emoji_style() -> String {
    "apple".to_string()
}

fn default_giant_emote_align() -> String {
    "center".to_string()
}

fn default_reply_style() -> String {
    "full".to_string()
}

fn default_emote_scale() -> f64 {
    1.0
}
fn default_emote_margin() -> f64 {
    0.125
}
fn default_emote_hover_size() -> u32 {
    96
}
fn default_deleted_message_style() -> String {
    "strikethrough".to_string()
}
fn default_username_separator() -> String {
    "none".to_string()
}
fn default_username_style() -> String {
    "plain".to_string()
}
fn default_username_accent_source() -> String {
    "user".to_string()
}
fn default_mod_action_style() -> String {
    "both".to_string()
}
fn default_mod_drag_layout() -> String {
    "column".to_string()
}
fn default_pinned_collapsed_style() -> String {
    "bar".to_string()
}
fn default_mod_pin_style() -> String {
    "both".to_string()
}

impl Default for ChatDesignSettings {
    fn default() -> Self {
        Self {
            show_dividers: false,
            alternating_backgrounds: false,
            // Twitch's own chat, measured: 14px text, 4px above and below each
            // message (spacing 8 renders as 8 / 2 per side). Emotes are 2em, so
            // 14px text also gives Twitch's 28px emotes.
            message_spacing: 8,
            font_size: 14,
            font_weight: 400,
            mention_color: "#ff4444".to_string(),
            reply_color: "#ff6b6b".to_string(),
            mention_animation: true,
            show_timestamps: false,
            show_timestamp_seconds: false,
            timestamp_format: default_timestamp_format(),
            emote_scale: 1.0,
            animate_emotes: default_animate_emotes(),
            show_chat_gifs: true,
            backfill_opacity: default_backfill_opacity(),
            emote_margin: 0.125,
            emote_hover_size: 96,
            deleted_message_style: "strikethrough".to_string(),
            hide_shared_chat: false,
            paint_mentions_in_body: true,
            compact_emote_tooltips: false,
            ffz_emote_effects: true,
            bttv_emote_modifiers: true,
            giant_emotes: true,
            user_card_opens_messages: false,
            seventv_emote_notices: true,
            link_previews: true,
            link_preview_keep_link: false,
            shorten_links: true,
            link_preview_trusted_domains: Vec::new(),
            username_separator: "none".to_string(),
            username_style: "plain".to_string(),
            name_color_adjustment: default_name_color_adjustment(),
            show_badges: true,
            badge_scale: 1.0,
            show_third_party_badges: true,
            hidden_badge_providers: Vec::new(),
            message_entrance: default_message_entrance(),
            emoji_style: default_emoji_style(),
            show_personal_emotes: true,
            giant_emote_align: default_giant_emote_align(),
            show_avatars: true,
            show_at_sign: false,
            reply_style: default_reply_style(),
            link_color: String::new(),
            link_underline: true,
            activity_font_size: None,
            username_colon: None,
            pinned_start_collapsed: None,
            polls_start_collapsed: None,
            username_accent_source: "user".to_string(),
            drag_moderation_enabled: true,
            mod_action_style: "both".to_string(),
            mod_drag_layout: "column".to_string(),
            pinned_collapsed_style: "bar".to_string(),
            mod_pin_style: "both".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LiveNotificationSettings {
    pub enabled: bool,
    pub play_sound: bool,
    #[serde(default)]
    pub sound_type: Option<String>,
    // Notification type toggles
    #[serde(default = "default_true")]
    pub show_live_notifications: bool,
    /// Go-live notifications for FAVOURITED channels, which may not be followed.
    /// Separate from `show_live_notifications` so a large favourites list can be
    /// silenced without losing notifications for the channels you follow.
    #[serde(default = "default_true")]
    pub show_favorite_live_notifications: bool,
    #[serde(default = "default_true")]
    pub show_whisper_notifications: bool,
    #[serde(default = "default_true")]
    pub show_update_notifications: bool,
    #[serde(default = "default_true")]
    pub show_drops_notifications: bool,
    #[serde(default = "default_true")]
    pub show_favorite_drops_notifications: bool,
    #[serde(default = "default_true")]
    pub show_channel_points_notifications: bool,
    #[serde(default = "default_true")]
    pub show_badge_notifications: bool,
    /// Gift subs you RECEIVE, polled off Twitch's own notification feed.
    /// Nothing else can see these: EventSub's gift subscription authorizes as
    /// the broadcaster, and Hermes' sub-gift topic is channel-scoped.
    #[serde(default = "default_true")]
    pub show_gift_sub_notifications: bool,
    /// Rewards Twitch tells your account about by name, off the same feed: a
    /// badge you earned, a drop reward waiting to be claimed. The only place
    /// that names them, whoever did the watching or the claiming.
    #[serde(default = "default_true")]
    pub show_twitch_reward_notifications: bool,
    // Notification method toggles (Dynamic Island vs Toast)
    #[serde(default = "default_true")]
    pub use_dynamic_island: bool,
    #[serde(default = "default_true")]
    pub use_toast: bool,
    // Native OS notifications (Windows/macOS)
    #[serde(default)]
    pub use_native_notifications: bool,
    #[serde(default = "default_true")]
    pub native_only_when_unfocused: bool,
    // Quick update: clicking update toast immediately starts update
    #[serde(default)]
    pub quick_update_on_toast: bool,
    // Toast placement: which screen anchor toasts appear at, and how far they
    // sit from the anchored top/bottom edge (raise the offset to lift toasts off
    // the chat input).
    #[serde(default = "default_toast_position")]
    pub toast_position: String,
    #[serde(default = "default_toast_edge_offset")]
    pub toast_edge_offset: u32,
    // Android background delivery. The in-app path needs a live WebView, so
    // these drive the WorkManager poll that keeps notifications arriving after
    // the app is closed. Desktop never reads them.
    #[serde(default = "default_true")]
    pub background_checks: bool,
    #[serde(default = "default_background_interval")]
    pub background_interval_minutes: u32,
    // Android push (FCM) registration. Desktop never reads it.
    #[serde(default = "default_true")]
    pub push_notifications: bool,
    // Channels excluded from live alerts, by login. Empty means every followed
    // channel notifies, which is the behaviour everyone already has, so an
    // upgrade changes nothing until someone opts a channel out.
    #[serde(default)]
    pub muted_live_channels: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// WorkManager's floor is 15 minutes and it will not schedule anything tighter.
fn default_background_interval() -> u32 {
    15
}

fn default_toast_position() -> String {
    "bottom-right".to_string()
}

fn default_toast_edge_offset() -> u32 {
    72
}

impl Default for LiveNotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            play_sound: true,
            sound_type: None,
            show_live_notifications: true,
            show_favorite_live_notifications: true,
            show_whisper_notifications: true,
            show_update_notifications: true,
            show_drops_notifications: true,
            show_favorite_drops_notifications: true,
            show_channel_points_notifications: true,
            show_badge_notifications: true,
            show_gift_sub_notifications: true,
            show_twitch_reward_notifications: true,
            use_dynamic_island: true,
            use_toast: true,
            use_native_notifications: false,
            native_only_when_unfocused: true,
            quick_update_on_toast: false,
            toast_position: "bottom-right".to_string(),
            toast_edge_offset: 72,
            background_checks: true,
            background_interval_minutes: 15,
            push_notifications: true,
            muted_live_channels: Vec::new(),
        }
    }
}

// Re-export DropsSettings from the drops module to avoid duplication
// The drops module has the complete struct with automation fields (priority_games, etc.)
pub use crate::models::drops::DropsSettings;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSwitchMode {
    #[default]
    SameCategory, // Switch to a stream in the same game/category
    FollowedStreams, // Switch to one of your live followed streamers
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AutoSwitchSettings {
    pub enabled: bool,
    #[serde(default)]
    pub mode: AutoSwitchMode, // What to switch to when stream goes offline
    pub show_notification: bool, // Show toast when auto-switching
    #[serde(default = "default_true")]
    pub auto_redirect_on_raid: bool, // Automatically follow raids to the target channel
    // Stay in the channel's chat when it goes offline instead of auto-switching.
    // The frontend has read this field for some time; without it here the
    // setting was dropped on every save.
    #[serde(default)]
    pub stay_in_offline_chat: bool,
}

impl Default for AutoSwitchSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: AutoSwitchMode::SameCategory,
            show_notification: true,
            auto_redirect_on_raid: true, // Enabled by default
            stay_in_offline_chat: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CompactViewPreset {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "isBuiltIn")]
    pub is_built_in: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CompactViewSettings {
    #[serde(rename = "selectedPresetId")]
    pub selected_preset_id: String,
    #[serde(rename = "customPresets", default)]
    pub custom_presets: Vec<CompactViewPreset>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MultiNookSlot {
    pub id: String,
    pub channel_login: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    pub volume: f32,
    pub muted: bool,
    pub is_focused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_minimized: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    /// Preferred Streamlink quality for this tile (set via the focused tile's gear
    /// menu). Without this field serde dropped the frontend's `quality` on the
    /// save round-trip, so per-tile quality reset to 'best' after every restart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Which platform this tile is on ("kick", "youtube", ...). Absent means
    /// Twitch, matching the bare-key convention in utils/providerKey.ts, so
    /// every grid saved before this field keeps working untouched.
    ///
    /// This struct is TYPED on `Settings` (`multi_nook_slots`), so it never
    /// reaches the flattened `extra` catch-all: a field the frontend sends but
    /// this struct does not name is silently dropped on save. See `quality`
    /// directly above, which is here for exactly that reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    pub quality: String,
    pub chat_placement: String,
    pub accounts: Vec<String>,
    pub current_account: String,
    pub hide_search_bar_on_startup: bool,
    pub discord_rpc_enabled: bool,
    pub video_player: VideoPlayerSettings,
    pub cache: CacheSettings,
    #[serde(default)]
    pub streamlink: StreamlinkSettings,
    #[serde(default)]
    pub drops: DropsSettings,
    #[serde(default)]
    pub favorite_streamers: Vec<String>,
    /// Display identity for the entries in `favorite_streamers`. Kept beside it
    /// rather than replacing it so every existing reader keeps working and no
    /// settings file in the field needs migrating.
    #[serde(default)]
    pub favorite_channels: Vec<FavoriteChannel>,
    #[serde(default)]
    pub chat_design: ChatDesignSettings,
    #[serde(default)]
    pub live_notifications: LiveNotificationSettings,
    #[serde(default)]
    pub last_seen_version: Option<String>,
    #[serde(default)]
    pub auto_switch: AutoSwitchSettings,
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Interface font id (see FONT_OPTIONS on the frontend). Persisted so the
    /// chosen font survives restarts; None falls back to the default font.
    #[serde(default)]
    pub font: Option<String>,
    /// User-typed interface font family, used when `font` is "custom".
    /// Ignored for any other font id.
    #[serde(default)]
    pub font_custom: Option<String>,
    /// Global glassiness, 0-100. Persisted so the slider survives restarts;
    /// None falls back to the default (100) on the frontend.
    #[serde(default)]
    pub glass_transparency: Option<u32>,
    #[serde(default)]
    pub setup_complete: bool,
    #[serde(default)]
    pub compact_view: Option<CompactViewSettings>,
    /// Whether diagnostic logging is enabled (defaults to true)
    #[serde(default = "default_true")]
    pub error_reporting_enabled: bool,
    /// Persisted multi-stream grid configurations
    #[serde(default)]
    pub multi_nook_slots: Vec<MultiNookSlot>,
    #[serde(default)]
    pub multi_nook_chat_hidden: bool,
    /// Whether the Moderator Logs pane is shown. Persisted so it survives app
    /// restarts and settings reloads instead of resetting to off each session.
    #[serde(default)]
    pub show_mod_logs: bool,
    /// Customizable keyboard shortcut overrides. Maps a bindable-command id to
    /// its user-assigned chord strings. Absent ids fall back to code defaults.
    #[serde(default)]
    pub keybindings: HashMap<String, Vec<String>>,
    /// Chat logging to plain text files (one folder per channel, one file per
    /// day), written by services::chat_logger_service.
    #[serde(default)]
    pub chat_logging: ChatLoggingSettings,
    /// What closing the main window does. Modelled here rather than left to
    /// `extra` because the window-event handler in main.rs reads it.
    #[serde(default)]
    pub close_to_tray: CloseToTrayMode,
    /// Channels followed inside StreamNook on platforms whose own follow list we
    /// can't read (Kick, TikTok). Modelled here rather than left to `extra`
    /// because the who's-live poller (provider_live_service) reads it.
    #[serde(default)]
    pub provider_follows: Vec<ProviderFollow>,
    /// Which YouTube live-chat view to read. Modelled here rather than left to
    /// `extra` because the YouTube adapter reads it when it resolves a stream.
    #[serde(default)]
    pub youtube_chat_view: YouTubeChatView,
    /// Streamers whose channels on different platforms are known to be the same
    /// person. Modelled here rather than left to `extra` because the link
    /// service WRITES it (a probe result), and an untyped field the backend
    /// writes is dropped by the next frontend save. `save_settings` re-stamps it
    /// from backend state for the same reason.
    #[serde(default)]
    pub channel_links: Vec<ChannelLinkGroup>,
    /// Merged cross-platform chat. Modelled here rather than left to `extra`
    /// because the link service reads `enabled` to decide whether to do any work
    /// at all.
    #[serde(default)]
    pub chat_blend: ChatBlendSettings,
    /// The command palette's snippets: the user's own entries, the ones they
    /// starred and their typed shortcuts. Kept here so a backup carries them and
    /// every window sees one copy.
    #[serde(default)]
    pub snippets: SnippetSettings,
    /// Catch-all for preference groups the frontend manages but this struct does
    /// not model field-by-field: highlight phrases, custom chat commands,
    /// moderation prefs, custom themes, the OLED accent, and any future ones.
    /// Without this, serde silently drops every key it doesn't recognize on save,
    /// so those settings never reached settings.json and reset on each restart.
    /// Flattening round-trips them verbatim, so the full frontend Settings shape
    /// persists across restarts and travels intact in exported backups.
    #[serde(flatten, default)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// A channel the user follows inside StreamNook, for platforms that don't
/// expose their own follow list to us. Mirrors the frontend `ProviderFollow`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFollow {
    pub provider: String,
    /// Slug / @handle / UC id — what chat and playback address.
    pub channel: String,
    #[serde(default)]
    pub display_name: Option<String>,
    /// Cached platform user id, filled on the first successful live check (Kick's
    /// batch live endpoint takes numeric ids, not slugs).
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub added_at: String,
    /// The user subscribes to this channel on the platform. Imported from the
    /// account sync; drives the subscriber marker in lists and the player.
    #[serde(default)]
    pub subscribed: bool,
    /// The channel's avatar, captured when the follow was imported.
    ///
    /// Both platforms hand us this in the import payload and it used to be
    /// discarded, after which the offline roster re-fetched the same image one
    /// channel at a time, on every app start. Stored here it costs nothing to
    /// draw. Absent for rows imported before this existed, and for any the
    /// import didn't carry — the per-card resolver still covers those.
    #[serde(default)]
    pub avatar: Option<String>,
    /// Imported from the platform's own follow list rather than added by hand
    /// here. A re-sync may remove these; hand-added follows are never touched.
    #[serde(default)]
    pub imported: bool,
}

/// One platform's half of a streamer who broadcasts in more than one place.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkMember {
    /// "twitch" | "kick" | "youtube".
    pub provider: String,
    /// Login / slug / UC id — whatever that platform's own live check accepts,
    /// never a YouTube video id, which names one broadcast and never returns.
    /// Stored with its original case: YouTube ids are case-sensitive.
    pub channel: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// The channels one streamer broadcasts on, so chat from all of them can be
/// merged into whichever one the viewer is actually watching.
///
/// A group is addressed by ANY of its members, not by a designated primary, so
/// the same link works whether the viewer arrives from Twitch or from Kick.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChannelLinkGroup {
    pub id: String,
    pub members: Vec<LinkMember>,
    /// Platforms the user said are NOT this streamer, so a probe never suggests
    /// them again. Stored as `provider:channel`.
    #[serde(default)]
    pub dismissed: Vec<String>,
}

/// One snippet the user wrote. Ids start `custom.` so they never collide with
/// the built-in library.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CustomSnippet {
    pub id: String,
    pub title: String,
    pub category: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct SnippetSettings {
    pub custom: Vec<CustomSnippet>,
    /// Starred snippet ids, built-in or custom.
    pub favorites: Vec<String>,
    /// Snippet id -> the lowercase shortcut that boosts it in the palette.
    pub aliases: std::collections::BTreeMap<String, String>,
}

/// Merging other platforms' chat into the normal chat panel.
#[derive(Serialize, Deserialize, Clone)]
pub struct ChatBlendSettings {
    /// Master switch. Off means nothing connects, nothing probes, and the
    /// single-channel path is untouched.
    #[serde(default)]
    pub enabled: bool,
    /// Which platforms may join a merged feed, by provider id. A platform
    /// absent from the map is allowed; only an explicit `false` excludes it.
    #[serde(default)]
    pub platforms: HashMap<String, bool>,
    /// Offer a link when a channel looks like it exists on another platform.
    /// Kick only — YouTube is never probed, because each lookup is a full
    /// watch-page fetch and a burst of those gets the IP challenged.
    #[serde(default = "default_true")]
    pub suggest_links: bool,
    /// Mark rows that came from a platform other than the one being watched.
    #[serde(default = "default_true")]
    pub show_platform_badge: bool,
}

impl Default for ChatBlendSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            platforms: HashMap::new(),
            suggest_links: true,
            show_platform_badge: true,
        }
    }
}

/// Identity for a favourited channel, so an unfollowed favourite can still be
/// drawn while it is offline (a name and a face; `favorite_streamers` is only
/// ids). Membership stays in `favorite_streamers` — this is a best-effort
/// display cache keyed by the SAME string, never a second answer to "is this
/// favourited".
///
/// Rows whose id has left `favorite_streamers` are ignored where they're read
/// rather than pruned, so nothing has to write settings during startup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FavoriteChannel {
    /// The key used in `favorite_streamers`: a Twitch numeric user id, or a
    /// composite `provider:channel`.
    pub id: String,
    /// "twitch" | "kick" | "youtube" | "tiktok".
    pub provider: String,
    /// Login / slug / @handle / UC id — what chat and playback address, and
    /// what the platform's live check accepts.
    pub channel: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub added_at: String,
}

fn default_theme() -> String {
    "winters-glass".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: "best".to_string(),
            chat_placement: "right".to_string(),
            accounts: vec![],
            current_account: "".to_string(),
            hide_search_bar_on_startup: true,
            discord_rpc_enabled: true,
            video_player: VideoPlayerSettings::default(),
            cache: CacheSettings::default(),
            streamlink: StreamlinkSettings::default(),
            drops: DropsSettings::default(),
            favorite_streamers: vec![],
            favorite_channels: vec![],
            provider_follows: vec![],
            youtube_chat_view: YouTubeChatView::default(),
            channel_links: vec![],
            chat_blend: ChatBlendSettings::default(),
            snippets: SnippetSettings::default(),
            chat_design: ChatDesignSettings::default(),
            live_notifications: LiveNotificationSettings::default(),
            last_seen_version: None,
            auto_switch: AutoSwitchSettings::default(),
            theme: default_theme(),
            font: None,
            font_custom: None,
            glass_transparency: None,
            setup_complete: false, // New users need to complete setup
            compact_view: None,
            error_reporting_enabled: true, // Diagnostics enabled by default
            multi_nook_slots: Vec::new(),
            multi_nook_chat_hidden: false,
            show_mod_logs: false,
            keybindings: HashMap::new(),
            chat_logging: ChatLoggingSettings::default(),
            close_to_tray: CloseToTrayMode::default(),
            extra: HashMap::new(),
        }
    }
}

/// One channel entry in the chat-logging allowlist. The shape matches the
/// frontend channel picker so the list round-trips with its display data;
/// only `channel_login` drives the filter.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct ChatLogChannel {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub channel_login: String,
    #[serde(default)]
    pub display_name: String,
}

/// What the main window's close button does.
///
/// `WithPopouts` is the long-standing behavior and stays the default: closing
/// hides to the tray only while MultiChat popouts are open, because quitting
/// would take them with it. The other two make the choice explicit.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CloseToTrayMode {
    /// Hide to the tray only when MultiChat popouts are open.
    #[default]
    WithPopouts,
    /// Always hide to the tray. Quit from the tray menu.
    Always,
    /// Always quit, even with popouts open.
    Never,
}

/// Which of YouTube's two live-chat views to read.
///
/// `Live` is the unfiltered firehose and stays the default. `Top` is what
/// youtube.com itself defaults to: YouTube drops messages it judges low quality
/// (and most of one author's repeats), so a very fast chat stays readable at the
/// cost of not seeing everything.
///
/// Measured by polling both views over the SAME window on one stream: Live 159
/// messages, Top 129, and every Top id was also in Live. So Top is a strict
/// subset that dropped about 19% here, not a different feed. Its join backlog is
/// smaller too (51 rows against 73).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum YouTubeChatView {
    /// Every message YouTube publishes.
    #[default]
    Live,
    /// YouTube's own filtered view.
    Top,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatLoggingSettings {
    /// Off by default: writing files to disk is the user's call.
    #[serde(default)]
    pub enabled: bool,
    /// Custom base folder; empty uses ChatLogs under the app data dir.
    #[serde(default)]
    pub folder: String,
    /// Channels to log; empty logs every channel the app has open.
    #[serde(default)]
    pub channels: Vec<ChatLogChannel>,
    /// Also log subscriptions, raids, announcements, and moderation actions.
    #[serde(default = "default_true")]
    pub include_events: bool,
    /// Start each line with the time it was sent.
    #[serde(default = "default_true")]
    pub timestamps: bool,
}

impl Default for ChatLoggingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            folder: String::new(),
            channels: Vec::new(),
            include_events: true,
            timestamps: true,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<Mutex<Settings>>,
    pub drops_service: Arc<TokioMutex<DropsService>>,
    pub background_service: Arc<TokioMutex<BackgroundService>>,
    pub layout_service: Arc<LayoutService>,
    pub emote_service: Arc<RwLock<EmoteService>>,
    /// Single owner of the Twitch web auth cookie. All code that previously
    /// scraped the cookie via `webview_cookie::read_twitch_web_auth_token`
    /// goes through this service instead.
    pub twitch_auth: TwitchAuthService,
    /// Out-of-process plugin host (spawn, supervise, capability and
    /// credential brokering). See docs/plugins/.
    pub plugin_host: Arc<crate::plugin_host::PluginHost>,
    /// Core parity watch heartbeat: one minute-watched per minute for the
    /// on-screen channel while it plays. See watch_heartbeat_service.rs.
    pub watch_heartbeat: Arc<crate::services::watch_heartbeat_service::WatchHeartbeatService>,
}

/// The gap used to be a plain number defaulting to 6.0, so every settings
/// file in the field carries a 6.0 that nobody chose. On a low-latency
/// channel that is a 7 s cushion against delivery that never pauses more
/// than a third of a second. Read the legacy default as "never set" so
/// those installs get the automatic per-path gap; a value anyone moved the
/// slider to survives untouched. Done on load like the avatar repair: a
/// one-field check, idempotent, persisted by the next ordinary save.
const LEGACY_LIVE_EDGE_GAP_DEFAULT: f32 = 6.0;

impl Settings {
    /// Overwrite the fields the BACKEND owns with the authoritative copies, so a
    /// save carrying the frontend's view of them cannot clobber the real values.
    ///
    /// Every field here is `#[serde(default)]`, which is what makes this
    /// necessary rather than merely tidy: a frontend save that omits the key
    /// deserializes it to an empty value, and the save then writes that emptiness
    /// to disk. `provider_follows` and `channel_links` lose entire user-made
    /// lists that way; `drops` loses automation state the plugin panel wrote.
    ///
    /// One owner on purpose. These used to be three separate assignments inside
    /// `save_settings`, which is exactly the shape where a fourth backend-owned
    /// field gets added and silently not re-stamped.
    pub fn adopt_backend_owned(&mut self, authoritative: &Settings) {
        self.provider_follows = authoritative.provider_follows.clone();
        self.drops = authoritative.drops.clone();
        self.channel_links = authoritative.channel_links.clone();
    }

    /// One-time flip of the low-latency engine to on for installs written before
    /// it became the default (see `low_latency_engine_defaulted`).
    pub fn enable_low_latency_engine_once(&mut self) {
        if !self.video_player.low_latency_engine_defaulted {
            self.video_player.experimental_low_latency = true;
            self.video_player.low_latency_engine_defaulted = true;
        }
    }

    pub fn retire_legacy_live_edge_gap(&mut self) {
        if self.video_player.ll_target_latency == Some(LEGACY_LIVE_EDGE_GAP_DEFAULT) {
            self.video_player.ll_target_latency = None;
        }
    }
}

#[cfg(test)]
mod backup_persistence_tests {
    use super::*;

    #[test]
    fn the_engine_turns_on_once_and_a_later_off_stays_off() {
        // An install from before the default flipped: off, never defaulted.
        let mut old = Settings::default();
        old.video_player.experimental_low_latency = false;
        old.video_player.low_latency_engine_defaulted = false;
        old.enable_low_latency_engine_once();
        assert!(old.video_player.experimental_low_latency);
        assert!(old.video_player.low_latency_engine_defaulted);
        // The viewer turns it off afterwards: the next load leaves it alone.
        old.video_player.experimental_low_latency = false;
        old.enable_low_latency_engine_once();
        assert!(!old.video_player.experimental_low_latency);
        // A file without either field parses to on.
        let mut json = serde_json::to_value(Settings::default()).expect("serialize");
        let vp = json["video_player"].as_object_mut().expect("video_player object");
        vp.remove("experimental_low_latency");
        vp.remove("low_latency_engine_defaulted");
        let mut parsed: Settings = serde_json::from_value(json).expect("parse");
        parsed.enable_low_latency_engine_once();
        assert!(parsed.video_player.experimental_low_latency);
    }

    #[test]
    fn legacy_default_gap_becomes_automatic_but_a_chosen_gap_survives() {
        let mut s = Settings::default();
        s.video_player.ll_target_latency = Some(6.0);
        s.retire_legacy_live_edge_gap();
        assert_eq!(s.video_player.ll_target_latency, None);

        let mut chosen = Settings::default();
        chosen.video_player.ll_target_latency = Some(3.2);
        chosen.retire_legacy_live_edge_gap();
        assert_eq!(chosen.video_player.ll_target_latency, Some(3.2));

        // A file written before the field existed at all parses to automatic.
        let mut json = serde_json::to_value(Settings::default()).expect("serialize");
        json["video_player"].as_object_mut().expect("video_player object").remove("ll_target_latency");
        let parsed: Settings = serde_json::from_value(json).expect("settings without the field parse");
        assert_eq!(parsed.video_player.ll_target_latency, None);
    }

    /// A modeled top-level preference must survive the save/load round-trip, and
    /// must NOT be swallowed by the flattened `extra` map on the way back.
    #[test]
    fn youtube_chat_view_round_trips() {
        let mut s = Settings::default();
        assert_eq!(s.youtube_chat_view, YouTubeChatView::Live, "firehose is the default");

        s.youtube_chat_view = YouTubeChatView::Top;
        let value = serde_json::to_value(&s).expect("serialize");
        assert_eq!(
            value.get("youtube_chat_view").and_then(|v| v.as_str()),
            Some("top"),
            "kebab-case on the wire, matching the TS YouTubeChatView union",
        );
        assert!(
            value.get("extra").is_none(),
            "modeled fields serialize at the top level",
        );

        let back: Settings = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.youtube_chat_view, YouTubeChatView::Top);
        assert!(!back.extra.contains_key("youtube_chat_view"));
    }

    /// A settings.json written before this setting existed must load, and land on
    /// the default rather than failing the whole parse.
    #[test]
    fn youtube_chat_view_defaults_when_absent() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .remove("youtube_chat_view");
        let loaded: Settings = serde_json::from_value(value).expect("older settings still load");
        assert_eq!(loaded.youtube_chat_view, YouTubeChatView::Live);
    }

    fn a_group(members: &[(&str, &str)]) -> ChannelLinkGroup {
        ChannelLinkGroup {
            id: "g1".into(),
            members: members
                .iter()
                .map(|(provider, channel)| LinkMember {
                    provider: (*provider).into(),
                    channel: (*channel).into(),
                    display_name: None,
                    avatar: None,
                })
                .collect(),
            dismissed: vec![],
        }
    }

    /// A save carrying the frontend's (empty) view of a backend-owned field must
    /// not erase it. Every one of these is `#[serde(default)]`, so an omitted key
    /// arrives as an empty list rather than as "leave this alone" — which is what
    /// makes losing a user's entire follow or link list a one-line mistake.
    #[test]
    fn a_frontend_save_cannot_wipe_backend_owned_fields() {
        let mut authoritative = Settings::default();
        authoritative.channel_links = vec![a_group(&[("twitch", "xqc"), ("kick", "xqc")])];
        authoritative.provider_follows = vec![ProviderFollow {
            provider: "kick".into(),
            channel: "xqc".into(),
            display_name: None,
            user_id: None,
            added_at: String::new(),
            subscribed: false,
            avatar: None,
            imported: false,
        }];

        // What a frontend save looks like: it never knew about either list.
        let mut incoming = Settings::default();
        assert!(incoming.channel_links.is_empty());
        incoming.adopt_backend_owned(&authoritative);

        assert_eq!(incoming.channel_links.len(), 1, "the link survived the save");
        assert_eq!(incoming.provider_follows.len(), 1, "the follow survived too");
    }

    /// YouTube channel ids are case-SENSITIVE, and the link member is handed
    /// straight to acquireChannel rather than rebuilt from a (lowercased) slice
    /// key. A settings round trip that folded the case would point chat at a
    /// channel that does not exist, with no error.
    #[test]
    fn a_youtube_link_member_keeps_its_case() {
        let mut s = Settings::default();
        s.channel_links = vec![a_group(&[("youtube", "UCxvT6Dy8OjXlpLFhCAyPWlQ")])];

        let value = serde_json::to_value(&s).expect("serialize");
        assert!(value.get("extra").is_none(), "modeled fields stay top level");
        assert!(
            value.get("channel_links").is_some(),
            "channel_links must be a typed field, never swallowed by the catch-all",
        );

        let back: Settings = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back.channel_links[0].members[0].channel, "UCxvT6Dy8OjXlpLFhCAyPWlQ");
        assert!(!back.extra.contains_key("channel_links"));
    }

    /// Blend is off unless the user turns it on, and a settings file written
    /// before it existed must load rather than failing the whole parse.
    #[test]
    fn chat_blend_defaults_to_off_and_tolerates_an_older_file() {
        assert!(!Settings::default().chat_blend.enabled, "off by default");
        assert!(Settings::default().chat_blend.suggest_links);

        let mut value = serde_json::to_value(Settings::default()).expect("serialize");
        let obj = value.as_object_mut().expect("object");
        obj.remove("chat_blend");
        obj.remove("channel_links");
        let parsed: Settings = serde_json::from_value(value).expect("an older file still parses");
        assert!(!parsed.chat_blend.enabled);
        assert!(parsed.channel_links.is_empty());
    }

    /// Frontend-managed preference groups the struct doesn't model (highlight
    /// phrases, custom themes, the OLED accent, ...) must survive a save/load
    /// round-trip through the flattened `extra` map instead of being dropped,
    /// and must serialize back at the top level (not nested under "extra").
    #[test]
    fn snippets_are_modelled_and_survive_a_round_trip() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value["snippets"] = serde_json::json!({
            "custom": [{ "id": "custom.gg.ab12", "title": "gg", "category": "Hype", "content": "GG" }],
            "favorites": ["custom.gg.ab12", "classic.kappa"],
            "aliases": { "custom.gg.ab12": "gg" }
        });
        let parsed: Settings = serde_json::from_value(value).unwrap();
        assert!(!parsed.extra.contains_key("snippets"));
        assert_eq!(parsed.snippets.custom[0].title, "gg");
        assert_eq!(parsed.snippets.favorites.len(), 2);
        let back = serde_json::to_value(&parsed).unwrap();
        assert_eq!(back["snippets"]["aliases"]["custom.gg.ab12"], "gg");
        assert!(back["snippets"]["custom"][0].get("keywords").is_none());
    }

    #[test]
    fn unknown_keys_round_trip_through_extra() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let obj = value.as_object_mut().expect("settings is an object");
        obj.insert(
            "chat_highlights".into(),
            serde_json::json!({ "phrases": ["raid", "gifted"] }),
        );
        obj.insert("oled_accent".into(), serde_json::json!("#ff9933"));

        let parsed: Settings = serde_json::from_value(value).expect("deserialize with extras");
        assert!(parsed.extra.contains_key("chat_highlights"));
        assert_eq!(
            parsed.extra.get("oled_accent").and_then(|v| v.as_str()),
            Some("#ff9933")
        );

        let reserialized = serde_json::to_value(&parsed).expect("serialize back");
        let out = reserialized.as_object().expect("object");
        assert!(out.contains_key("chat_highlights"));
        assert!(out.contains_key("oled_accent"));
        assert!(!out.contains_key("extra"));
    }

    /// A full settings dump with no unrecognized keys round-trips with an empty
    /// catch-all and emits no stray "extra" wrapper key (backward compatible).
    /// The readable-name-color mode and the four chat_design keys the struct
    /// never named (they were being dropped on every save) must survive a
    /// round trip. The optional ones stay ABSENT when unset, so a fresh
    /// install keeps the frontend default rather than a value written by save.
    #[test]
    fn chat_design_name_color_and_optional_keys_round_trip() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let design = value["chat_design"].as_object_mut().expect("chat_design object");
        assert_eq!(design.get("name_color_adjustment").and_then(|v| v.as_str()), Some("hsl_loop"));
        assert!(design.get("activity_font_size").is_none(), "absent stays absent");
        design.insert("name_color_adjustment".into(), serde_json::json!("off"));
        design.insert("activity_font_size".into(), serde_json::json!(16));
        design.insert("username_colon".into(), serde_json::json!(true));
        design.insert("pinned_start_collapsed".into(), serde_json::json!(true));
        design.insert("polls_start_collapsed".into(), serde_json::json!(false));

        let parsed: Settings = serde_json::from_value(value).expect("deserialize");
        assert_eq!(parsed.chat_design.name_color_adjustment, "off");
        assert_eq!(parsed.chat_design.activity_font_size, Some(16));
        assert_eq!(parsed.chat_design.username_colon, Some(true));
        assert_eq!(parsed.chat_design.pinned_start_collapsed, Some(true));
        assert_eq!(parsed.chat_design.polls_start_collapsed, Some(false));

        let again = serde_json::to_value(&parsed).expect("re-serialize");
        assert_eq!(again["chat_design"]["activity_font_size"], serde_json::json!(16));
        assert_eq!(again["chat_design"]["name_color_adjustment"], serde_json::json!("off"));
    }

    #[test]
    fn default_settings_round_trip_with_empty_extra() {
        let original = Settings::default();
        let json = serde_json::to_string(&original).expect("serialize");
        assert!(!json.contains("\"extra\""));

        let parsed: Settings = serde_json::from_str(&json).expect("deserialize");
        assert!(parsed.extra.is_empty());
        assert_eq!(parsed.theme, original.theme);
    }

    /// MultiNook presets are a frontend-managed key the struct does not model
    /// field-by-field; they must round-trip through `extra` so they persist across
    /// restarts AND ride along in exported settings backups (export_settings
    /// serializes this exact shape, minus the non-portable session keys). Every
    /// nested field (per-channel quality, the icon) must survive verbatim.
    #[test]
    fn multi_nook_presets_round_trip_through_extra() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let obj = value.as_object_mut().expect("settings is an object");
        obj.insert(
            "multi_nook_presets".into(),
            serde_json::json!([
                {
                    "id": "preset-1",
                    "name": "FNCS",
                    "channels": [
                        { "channelLogin": "mande", "channelName": "Mande", "quality": "720p60" }
                    ],
                    "icon": { "type": "game", "imageUrl": "https://example/boxart.jpg", "label": "Fortnite" },
                    "createdAt": 1,
                    "updatedAt": 2
                }
            ]),
        );

        let parsed: Settings = serde_json::from_value(value).expect("deserialize with presets");
        assert!(parsed.extra.contains_key("multi_nook_presets"));

        let reserialized = serde_json::to_value(&parsed).expect("serialize back");
        let out = reserialized.as_object().expect("object");
        // Re-emitted at the top level (so export_settings carries it), not nested under "extra".
        assert!(!out.contains_key("extra"));
        let presets = out
            .get("multi_nook_presets")
            .and_then(|v| v.as_array())
            .expect("presets array survived");
        let first = presets[0].as_object().expect("preset object");
        assert_eq!(first.get("name").and_then(|v| v.as_str()), Some("FNCS"));
        assert!(first.get("icon").is_some());
        let chan = first["channels"][0].as_object().expect("channel object");
        assert_eq!(chan.get("quality").and_then(|v| v.as_str()), Some("720p60"));
    }

    /// A slot's `provider` must survive the save round trip. `multi_nook_slots`
    /// is a TYPED field, so unlike presets it does NOT ride the flattened
    /// `extra` catch-all: any key this struct does not name is dropped on save.
    /// That already happened once with `quality`, and a provider lost here would
    /// look fine in testing and silently turn every non-Twitch tile back into a
    /// Twitch one after a restart.
    #[test]
    fn multi_nook_slot_provider_round_trips() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let obj = value.as_object_mut().expect("settings is an object");
        obj.insert(
            "multi_nook_slots".into(),
            serde_json::json!([
                {
                    "id": "cell-1",
                    "channelLogin": "xqc",
                    "volume": 1.0,
                    "muted": false,
                    "isFocused": true,
                    "provider": "kick"
                }
            ]),
        );

        let parsed: Settings = serde_json::from_value(value).expect("deserialize with slots");
        assert_eq!(
            parsed.multi_nook_slots[0].provider.as_deref(),
            Some("kick"),
            "provider must deserialize onto the typed slot"
        );

        let reserialized = serde_json::to_value(&parsed).expect("serialize back");
        let slots = reserialized
            .get("multi_nook_slots")
            .and_then(|v| v.as_array())
            .expect("slots array survived");
        let slot = slots[0].as_object().expect("slot object");
        assert_eq!(
            slot.get("provider").and_then(|v| v.as_str()),
            Some("kick"),
            "provider must survive serialization back to disk"
        );
    }

    /// A grid saved before the provider field existed must keep loading, with
    /// the absent provider meaning Twitch (the bare-key convention).
    #[test]
    fn multi_nook_slot_without_provider_still_loads() {
        let mut value = serde_json::to_value(Settings::default()).expect("serialize defaults");
        let obj = value.as_object_mut().expect("settings is an object");
        obj.insert(
            "multi_nook_slots".into(),
            serde_json::json!([
                { "id": "cell-1", "channelLogin": "xqc", "volume": 1.0, "muted": false, "isFocused": true }
            ]),
        );

        let parsed: Settings = serde_json::from_value(value).expect("legacy slot must deserialize");
        assert_eq!(parsed.multi_nook_slots[0].provider, None);
        let reserialized = serde_json::to_value(&parsed).expect("serialize back");
        let slot = reserialized["multi_nook_slots"][0]
            .as_object()
            .expect("slot object");
        assert!(
            !slot.contains_key("provider"),
            "an absent provider must not be written back as null"
        );
    }
}

fn default_timestamp_format() -> String {
    "12h".to_string()
}

fn default_animate_emotes() -> String {
    "always".to_string()
}

fn default_backfill_opacity() -> u32 {
    100
}
