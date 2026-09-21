import type { ProviderId } from './providers';

export interface AudioBoostSettings {
  enabled: boolean;
  gain: number; // Makeup gain multiplier applied after compression (1 = unity)
  threshold: number; // dB, level where compression begins
  knee: number; // dB, how gradually compression ramps in around the threshold
  ratio: number; // x:1 compression ratio above the threshold
  attack: number; // seconds to clamp down once over the threshold
  release: number; // seconds to ease back off once under the threshold
}

// Gentle, pleasant defaults: enabling the feature with these values gives an
// immediately nicer, slightly louder and more even mix that the user can tweak.
// Off by default so playback is never touched until the user opts in.
export const DEFAULT_AUDIO_BOOST: AudioBoostSettings = {
  enabled: false,
  gain: 1.5,
  threshold: -30,
  knee: 30,
  ratio: 6,
  attack: 0.003,
  release: 0.25,
};

export interface SongIdSettings {
  // Seconds of audio to fingerprint. Longer is more robust (especially over
  // talking or noise) at the cost of a longer wait before the result.
  capture_seconds: number;
  // Extra capture + lookup attempts when the first finds nothing. Each retry
  // listens again, so a missed window (ad break, quiet moment) gets another shot.
  retries: number;
}

// 10s captures comfortably beat the ~5s minimum for a confident match; one retry
// rescues the occasional miss without a long wait.
export const DEFAULT_SONG_ID: SongIdSettings = {
  capture_seconds: 10,
  retries: 1,
};

export interface VideoPlayerSettings {
  max_buffer_length: number;
  autoplay: boolean;
  muted: boolean;
  volume: number;
  start_quality: number;
  lock_aspect_ratio: boolean;
  cinema_mode?: boolean;
  audio_boost?: AudioBoostSettings;
  song_id?: SongIdSettings;
  experimental_low_latency?: boolean;
  ll_target_latency?: number;
  /** Ad-free live playback. Android only; the desktop app resolves through its
   *  plugin seam and ignores both of these. */
  ad_bypass_enabled?: boolean;
  /** Relay bases to prefer, comma or newline separated. Empty = bundled pool. */
  ad_bypass_proxies?: string;
  /**
   * What leaving the app does while a stream plays. Android only.
   *
   * 'pip'   - float the video in a system picture-in-picture window (default).
   * 'audio' - no floating window; drop to the audio-only rendition and keep
   *           playing behind a media notification that taps back into the app.
   */
  background_mode?: 'pip' | 'audio';
  /** Scroll over the player to adjust volume. Default true. */
  scroll_volume?: boolean;
  /** Scroll down over the player to open the channel About drawer. Default
   *  true. Moves to Shift + scroll down while `scroll_volume` is also on. */
  scroll_about_reveal?: boolean;
  /** Middle-click the player to toggle mute. Default true. */
  middle_click_mute?: boolean;
  /** Volume moved per wheel notch, 0.01-0.25. Default 0.05. */
  wheel_volume_step?: number;
  /** Reopen a VOD where you left off. Default true. Typed in Rust. */
  resume_vod_playback?: boolean;
}

export interface CacheSettings {
  enabled: boolean;
  expiry_days: number;
}

export interface StreamlinkSettings {
  stream_timeout: number;          // Native retry budget (seconds)
  retry_streams: number;           // Native retry delay between attempts (seconds)
  /** Request h265 + AV1 in addition to h264. Unlocks Twitch Enhanced
   *  Broadcasting tiers (1440p60 AV1, 720p60 HEVC). Default on. */
  enhanced_codecs?: boolean;
}

export type RecoveryMode = 'Automatic' | 'Relaxed' | 'ManualOnly';

export interface RecoverySettings {
  recovery_mode?: RecoveryMode;
  stale_progress_threshold_seconds?: number;
  streamer_blacklist_duration_seconds?: number;
  campaign_deprioritize_duration_seconds?: number;
  detect_game_category_change?: boolean;
  notify_on_recovery_action?: boolean;
  max_recovery_attempts?: number;
}

export interface PriorityChannel {
  channel_id: string;
  channel_login: string;
  display_name: string;
}

// Chat logging to plain text files: one folder per channel, one file per day.
// Mirrors ChatLoggingSettings on the Rust side, which does the writing.
export interface ChatLoggingSettings {
  enabled?: boolean; // Off by default
  folder?: string; // Custom base folder; empty uses ChatLogs under the app data dir
  channels?: PriorityChannel[]; // Channels to log; empty logs every channel you open
  include_events?: boolean; // Also log subs, raids, announcements, and moderation actions (default: true)
  timestamps?: boolean; // Start each line with the time it was sent (default: true)
}

export interface DropsSettings {
  auto_claim_drops: boolean;
  auto_claim_channel_points: boolean;
  notify_on_drop_available: boolean;
  notify_on_drop_claimed: boolean;
  notify_on_points_claimed: boolean;
  check_interval_seconds: number;
  // Automation settings
  automation_enabled?: boolean;
  priority_games?: string[];
  excluded_games?: string[];
  priority_mode?: 'PriorityOnly' | 'EndingSoonest' | 'LowAvailFirst';
  watch_interval_seconds?: number;
  // Watch token allocation settings
  reserve_token_for_current_stream?: boolean; // Reserve one watch token for current stream (default: true)
  auto_reserve_on_watch?: boolean; // Automatically reserve token when starting a stream (default: true)
  priority_channels?: PriorityChannel[]; // Channels to prioritize for channel points automation
  prefer_favorites?: boolean; // Collect your live favorited channels instead of the priority list (default: false)
  // Recovery settings
  recovery_settings?: RecoverySettings;
}

export interface DropChannel {
  id: string;
  name: string;
  display_name: string;
  game_name: string;
  viewer_count: number;
  is_live: boolean;
  drops_enabled: boolean;
}

export interface CurrentDropInfo {
  campaign_id: string;
  campaign_name: string;
  drop_id: string;
  drop_name: string;
  drop_image?: string; // Image URL from benefit_edges
  required_minutes: number;
  current_minutes: number;
  game_name: string;
}

export interface DropsDeviceCodeInfo {
  user_code: string;
  verification_uri: string;
  device_code: string;
  interval: number;
  expires_in: number;
}

export interface DropProgressStatus {
  active: boolean;
  // Every watch-time reward for the watched game/campaign is already earned, so
  // there is nothing left to farm. Distinct from `active: false` (nothing being
  // watched): complete means "done here", so the UI shows a finished state
  // rather than reverting to the idle gift.
  complete?: boolean;
  current_channel: DropChannel | null;
  current_campaign: string | null;
  current_drop: CurrentDropInfo | null;
  eligible_channels: DropChannel[];
  last_update: string;
}

export interface ChatDesignSettings {
  show_dividers: boolean;
  alternating_backgrounds: boolean;
  message_spacing: number; // 0-20 pixels
  font_size: number; // 10-48 pixels
  font_weight: number; // 300-700
  activity_font_size?: number; // 10-28 pixels (MultiChat activity feed rows)
  mention_color: string; // Hex color for @ mentions
  reply_color: string; // Hex color for reply threads
  mention_animation: boolean; // Enable red-shift animation for mentions
  show_timestamps?: boolean; // Show timestamp next to each message
  show_timestamp_seconds?: boolean; // Include seconds in timestamps
  timestamp_format?: '12h' | '24h'; // Formatted in Rust; default 12h
  emote_scale?: number; // Emote size multiplier (0.5x to 3x). Default 1.
  // Animated emotes: play always (default), only while the row is hovered,
  // or never (first frame). Typed in Rust too; the CDNs serve static files.
  animate_emotes?: 'always' | 'hover' | 'never';
  // Twitch chat GIFs (Tier 2/3 subscribers). Off swaps each for a chip that
  // reveals on click. Typed in Rust too (ChatDesignSettings). Default true.
  show_chat_gifs?: boolean;
  // Opacity of history rows loaded on join, 0-100. Default 100.
  backfill_opacity?: number;
  emote_margin?: number; // Horizontal margin around emotes in rem. Negative values overlap. Default 0.125.
  // Height in pixels of the enlarged emote shown in the hover preview card.
  // Default 96 (one step up from the original fixed 64px preview).
  emote_hover_size?: number;
  // How deleted/moderated messages render. 'strikethrough' is the prior
  // default behavior. 'hidden' fully suppresses the row. 'dimmed' reduces
  // opacity without strike. 'keep' leaves the message rendered as if nothing
  // happened (useful for mods auditing what was said).
  deleted_message_style?: 'strikethrough' | 'hidden' | 'dimmed' | 'keep';
  // Suppress messages flagged as originating from another room in a Twitch
  // shared-chat session. Default false (keep them visible).
  hide_shared_chat?: boolean;
  // Toggle the inline paint render on @mentions in message bodies. Default
  // true (preserves prior always-painted behavior). Off renders the mention
  // chip in the mentioned user's flat color only.
  paint_mentions_in_body?: boolean;
  // Use compact emote tooltips (just the name) instead of the upscaled
  // emote preview. Default false (preview tooltip is the current behavior).
  compact_emote_tooltips?: boolean;
  // Apply FrankerFaceZ modifier effects (ffzW wide, ffzX/Y flips, ffzRainbow,
  // ...) to the preceding emote, like FFZ's own client. Default true. Off
  // renders modifier emotes as plain overlay emotes instead.
  ffz_emote_effects?: boolean;
  // Apply BetterTTV modifier effects (w! wide, h!/v! flips, c! cursed, p!
  // party, s! shake, l!/r! rotate, z! zero space) to the emote AFTER them,
  // like BetterTTV's own client. Default true. Off renders the modifiers as
  // plain emotes.
  bttv_emote_modifiers?: boolean;
  // Render the last emote of a "Gigantify an Emote" power-up message (msg-id
  // gigantified-emote-message) at 4x below the message body, like Twitch does.
  // Default true. Off renders the emote inline at its normal size.
  giant_emotes?: boolean;
  // Which half of the user card opens first: their recent messages (default) or
  // the profile body. The card switches between the two either way.
  user_card_opens_messages?: boolean;
  // Show an in-chat notice when a channel's 7TV emote set changes live (a mod
  // adds, removes, or renames an emote). Default true.
  seventv_emote_notices?: boolean;
  // Auto-expand inline preview cards for links from trusted domains (YouTube,
  // Twitch, imgur, etc.). Other links stay plain clickable links. Default true.
  link_previews?: boolean;
  // When a link has a preview card, also keep the inline link visible in the
  // message ("Card + link"). When false, the inline link is hidden and only the
  // card is shown ("Clean"). Default false (clean). Ignored when previews are off.
  link_preview_keep_link?: boolean;
  // Show links as a compact host + truncated path label instead of the full
  // raw URL (the full URL stays the click target and hover tooltip). Default true.
  shorten_links?: boolean;
  // Registrable hosts the user has chosen to trust (auto-expand their previews),
  // in addition to the built-in allowlist. Stored stripped of a leading `www.`
  // so an entry matches every subdomain. Managed from Chat settings.
  link_preview_trusted_domains?: string[];
  // When the pinned message is collapsed, 'bar' shows a thin one-line bar (sender
  // + truncated text) you can click to expand; 'hidden' keeps the prior behavior
  // where only the header pin icon remains. Default 'bar'.
  pinned_collapsed_style?: 'bar' | 'hidden';
  // Pinned messages start collapsed to the compact bar when entering a channel;
  // expanding is a per-channel choice that lasts until you switch. Default true.
  pinned_start_collapsed?: boolean;
  // Live polls open collapsed to their header instead of expanded. Independent
  // of `show_polls`, which hides the card entirely. Default false (polls open).
  polls_start_collapsed?: boolean;
  // --- Username prefix styling (normal messages only; action/"/me" stay plain) ---
  // Glyph rendered between the username and the message body.
  username_separator?: 'none' | 'colon' | 'dot' | 'arrow' | 'pipe' | 'dash';
  // How the username itself is emphasized as a prefix: plain, a thin accent bar
  // before it, a frosted chip/tag, bracketed [name], or a small color dot.
  username_style?: 'plain' | 'bar' | 'chip' | 'brackets' | 'dot';
  // Color source for the separator glyph, accent bar, dot, brackets, and chip
  // tint: the chatter's own color, or the active theme accent.
  username_accent_source?: 'user' | 'theme';
  // Deprecated: superseded by username_separator. Kept so an existing "colon on"
  // setting migrates to username_separator: 'colon'.
  username_colon?: boolean;
  // Readable name colors: 'hsl_loop' (default) nudges a chatter's color
  // lighter on a dark theme (darker on a light one) until it stands out,
  // keeping the hue; 'off' shows colors exactly as set.
  name_color_adjustment?: 'off' | 'hsl_loop';
  // --- Badges ---
  // Native platform badges (Twitch, Kick, YouTube). Default true.
  show_badges?: boolean;
  // Badge size multiplier, 0.5-2.5. Default 1.
  badge_scale?: number;
  // 7TV, FFZ, Chatterino and the other add-on badges. Default true.
  show_third_party_badges?: boolean;
  // Add-on badge providers to leave out (lowercase ids: 'streamnook', '7tv',
  // 'ffz', 'bttv', 'chatterino', 'homies', 'moltorino', 'chatsen', 'chatty',
  // 'dankchat'). Default none.
  hidden_badge_providers?: string[];
  // How a new message arrives: none (default), fade, slide, rise. History
  // rows loaded on join never animate.
  message_entrance?: 'none' | 'fade' | 'slide' | 'rise';
  // Which emoji set draws emoji in messages. Default 'apple'; 'system' uses
  // the device font.
  emoji_style?: 'system' | 'apple' | 'google' | 'twitter' | 'facebook';
  // Show 7TV personal emotes (off renders their text). Default true.
  show_personal_emotes?: boolean;
  // Where a gigantified emote sits under the message; 'inline' keeps it in
  // the text at normal size. Default 'center'.
  giant_emote_align?: 'left' | 'center' | 'right' | 'inline';
  // Profile pictures beside names on platforms that send them (YouTube,
  // TikTok). Default true.
  show_avatars?: boolean;
  // Put an @ before every name. Default false.
  show_at_sign?: boolean;
  // How a reply shows its parent: a context line (default), an @name in the
  // body, or nothing.
  reply_style?: 'full' | 'mention' | 'off';
  // Link color (hex); empty uses the theme's link color. Default empty.
  link_color?: string;
  // Underline links. Default true.
  link_underline?: boolean;
  // How per-message mod actions are offered in the chat hover dock:
  // 'buttons' = the classic click buttons (delete/timeout/ban), 'drag' = the
  // grab handle that lifts a chatter into action buckets, 'both' = offer both.
  // Default 'both'. Profile/whisper buckets are available to everyone whenever
  // the grip is shown; delete/timeout/ban are mod-only either way.
  mod_action_style?: 'buttons' | 'drag' | 'both';
  // Style of the drag-to-moderate controls: 'column' = vertical bucket column
  // docked beside chat; 'bar' = horizontal bucket row above the chat. Default
  // 'column'. (A legacy 'slider' value may persist from older builds; it is
  // treated as 'column'.)
  mod_drag_layout?: 'column' | 'bar';
  // Where the moderator Pin action appears: 'inline' = a quick button next to
  // Copy on the message, 'drag' = a drop target in the drag-to-moderate gesture,
  // 'both' = both places. Mod-only either way. Default 'both'.
  mod_pin_style?: 'inline' | 'drag' | 'both';
  // Deprecated: superseded by mod_action_style ('buttons' when this was false).
  drag_moderation_enabled?: boolean;
}

export interface HighlightPhrase {
  id: string;
  pattern: string;
  enabled: boolean;
  case_sensitive: boolean;
  whole_word: boolean;
  is_regex: boolean;
  color: string;
  // Optional sound played when the phrase matches. null/undefined = silent.
  sound_id?: string | null;
  // Per-phrase cooldown for the sound, in seconds. Default 3.
  cooldown_seconds?: number;
}

// A user-supplied audio file usable as a highlight sound. `id` is
// `file:<random>` so it can sit in the same field as the built-in tone ids.
export interface CustomSound {
  id: string;
  name: string;
  path: string;
}

export interface ChatHighlightSettings {
  phrases: HighlightPhrase[];
  custom_sounds?: CustomSound[];
  built_in?: BuiltInHighlightSettings;
  users?: HighlightUser[];
  badges?: HighlightBadge[];
  // Global appearance controls applied to every match (phrase / user / badge /
  // built-in event). Sound and title-flash still fire under 'none' — this only
  // governs the in-row visual.
  appearance?: HighlightAppearanceSettings;
}

export type HighlightDisplayStyle = 'standard' | 'minimal' | 'none';

export interface HighlightAppearanceSettings {
  display_style?: HighlightDisplayStyle; // default 'standard'
  opacity?: number; // 0-100, default 20. Modulates the row tint alpha only.
  // When true, any highlight match that arrives while the window is blurred
  // will flash document.title until the window regains focus. Default off.
  flash_title_when_unfocused?: boolean;
}

// Always-highlight specific users. Match by login (case-insensitive). The
// login is what we have at parse time without an extra Helix call, and it's
// stable enough for this UI use (users who change their name infrequently
// will just need the rule updated). User-id binding could be added later.
export interface HighlightUser {
  id: string; // rule id (UUID), not the Twitch user id
  enabled: boolean;
  username: string; // Twitch login, lowercased on save
  color: string;
  sound_id?: string | null;
  cooldown_seconds?: number;
}

// Highlight all messages from users carrying a specific badge. badge_key is
// the IRC tag format `name/version` (e.g. "moderator/1", "subscriber/12").
// A trailing /* matches any version (e.g. "subscriber/*" highlights every
// tier-1 sub regardless of tenure).
export interface HighlightBadge {
  id: string;
  enabled: boolean;
  badge_key: string;
  label?: string; // optional display name for the settings UI
  color: string;
  sound_id?: string | null;
  cooldown_seconds?: number;
}

// Built-in (flag-driven) message highlights. Each event is OFF by default to
// preserve the prior baseline; turning a row on applies a tinted background +
// left border in the configured color whenever the IRC tag is present.
export interface BuiltInHighlightRule {
  // First-time chatters only. 'wash' (default) is the tinted row with a left
  // bar; 'ring' outlines the row in the color instead.
  style?: 'wash' | 'ring';
  // 'ring' only: a faint color-matched fill inside the ring.
  fill?: boolean;
  // A one-shot glint when the row arrives: none (default), sheen, pulse, chase.
  animation?: 'none' | 'sheen' | 'pulse' | 'chase';
  animate_repeat?: boolean;
  enabled: boolean;
  color: string;
}

export interface BuiltInHighlightSettings {
  // first-msg=1 IRC tag. Already shipped pre-2026-05-24 with a hardcoded
  // purple gradient; this entry lets the user re-color (or disable) it.
  first_time_chatter?: BuiltInHighlightRule;
  // returning-chatter=1 IRC tag.
  returning_chatter?: BuiltInHighlightRule;
  // Messages where parsed.user_id === currentUser.user_id.
  self_message?: BuiltInHighlightRule;
  // USERNOTICE msg-id=raid (the raid announcement row itself).
  raider?: BuiltInHighlightRule;
}

// Per-user customization. nickname and color are independent: either can be set
// without the other. Keyed by Twitch user_id (stable across name changes).
export interface UserChatOverride {
  user_id: string;
  // Last-seen real username, captured purely so the Settings UI can show the
  // user "Bob → Robert" without re-fetching. Not load-bearing.
  username?: string;
  nickname?: string | null;
  color?: string | null;
}

export interface ChatCustomizationSettings {
  user_overrides?: Record<string, UserChatOverride>;
}

// User-defined slash command. Trigger is the bare word matched at the start
// (and optionally end) of a message. Expansion is the templated message body.
// See `expandUserCommand` in utils/chatCommands.ts for the full placeholder
// grammar — {N}, {N+}, {*}, {{ }}, and dotted fields (user.name, user.id,
// channel.name, channel.id, stream.title, stream.game, stream.uptime).
export interface UserSlashCommand {
  id: string;
  trigger: string;
  expansion: string;
  description?: string;
  enabled: boolean;
  // When true (default), the trigger only matches messages starting with `/`
  // and the leading slash is stripped before matching. When false, the trigger
  // matches plain text and the message body is rewritten in place.
  require_slash?: boolean;
  // When true, the trigger also matches at the END of a message (suffix mode),
  // not just the start. Useful for shortcut catchphrases. Default false.
  also_match_suffix?: boolean;
}

export interface ChatCommandsSettings {
  user_commands: UserSlashCommand[];
}

// ─────────────────────────────────────────────────────────────────────────
// Reminders — messages StreamNook auto-posts into a streamer's chat to nudge
// the broadcaster (or chat) about something. Each reminder pairs one trigger
// with the text to post. The firing engine lives in utils/reminderEngine.ts.
// ─────────────────────────────────────────────────────────────────────────

// The "avenue" that makes a reminder fire.
export type ReminderTriggerType =
  | 'interval' // repeat every N minutes while you're connected to the channel
  | 'delay'    // once, N minutes after you join the channel's chat
  | 'clock'    // once per day, at a local HH:MM time
  | 'uptime'   // once, when the watched stream has been live N minutes
  | 'keyword'; // when an incoming chat message matches a word or phrase

export type ReminderKeywordMatch = 'contains' | 'word' | 'exact';
export type ReminderKeywordFrom = 'anyone' | 'broadcaster' | 'mods';
// Whether a reminder follows whatever channel you're watching, or is pinned to
// one specific channel.
export type ReminderChannelScope = 'current' | 'specific';

export interface Reminder {
  id: string;
  enabled: boolean;
  // Friendly name shown in the list. Falls back to the message when empty.
  label?: string;
  // Text posted to chat. Supports the same {placeholder} grammar as custom
  // commands ({channel}, {stream.title}, {stream.uptime}, etc.) so it can, for
  // example, @mention the streamer.
  message: string;
  trigger: ReminderTriggerType;
  // How many copies to post per fire (1-5). Copies past the first carry the
  // invisible duplicate-bypass suffix so Twitch accepts the repeats.
  repeat_count?: number;
  // Which chat it posts in.
  channel_scope?: ReminderChannelScope;
  channel_login?: string;
  channel_id?: string;
  // Trigger parameters. Only the field matching `trigger` is read. Durations are
  // stored in SECONDS so any amount can be set; the older *_minutes fields are
  // still read as a fallback for reminders made before seconds support.
  interval_seconds?: number;
  delay_seconds?: number;
  clock_time?: string; // "HH:MM", 24-hour local time
  uptime_seconds?: number;
  /** @deprecated read-only fallback — new code writes interval_seconds */
  interval_minutes?: number;
  /** @deprecated read-only fallback — new code writes delay_seconds */
  delay_minutes?: number;
  /** @deprecated read-only fallback — new code writes uptime_seconds */
  uptime_minutes?: number;
  keyword?: string;
  keyword_match?: ReminderKeywordMatch;
  keyword_from?: ReminderKeywordFrom;
  // Smallest gap between keyword fires, so a busy chat can't spam it. Default 5.
  keyword_cooldown_minutes?: number;
}

export interface RemindersSettings {
  reminders: Reminder[];
}

// Chat input QoL options. Both default off — preserve prior behavior.
// 7TV / cosmetic visual controls. Today: paint drop-shadow render mode.
// Default 'all' preserves the prior behavior (whatever shadows the paint
// artist defined render in full). 'one' = first shadow only (faster + less
// busy). 'none' = no shadows (cleanest, best on busy backgrounds).
export interface CosmeticsSettings {
  paint_shadows?: 'all' | 'one' | 'none';
}

// Render / perf controls. All optional with defaults preserving prior behavior.
export interface ChatRenderSettings {
  // Animate the scroll when the user clicks the "Resume" button at the
  // bottom of the chat (a brisk eased glide back to the live bottom).
  // Defaults to on. Auto-scroll on new messages stays instant either way —
  // smooth-scrolling on every PRIVMSG would fight itself in fast chats.
  smooth_scroll_on_resume?: boolean;
  // Max messages held in the local buffer per channel. Default 100.
  // Range 50-1000. Larger = more scrollback at memory cost.
  message_buffer_cap?: number;
}

export interface ImageUploaderSettings {
  enabled?: boolean;
  // A preset id from utils/imageUploadHosts (nuuls, catbox, litterbox, uguu)
  // or "custom" for the fields below.
  preset?: string;
  // Extra text form fields some hosts require, url-encoded "k=v&k2=v2".
  extra_fields?: string;
  // Multipart POST target. Chatterino's default is https://i.nuuls.com/upload.
  url?: string;
  // Form field name for the file (nuuls: "attachment").
  form_field?: string;
  // Dotted path into a JSON response for the link; empty = the body is the link.
  response_path?: string;
}

export interface ChatInputSettings {
  // Append an invisible suffix when sending the same message twice in a row,
  // so Twitch's duplicate-message rejection doesn't eat the second send.
  bypass_duplicate?: boolean;
  // Paste an image into the composer to upload it and insert the link. Off by
  // default: it sends the image to a third-party host you choose.
  image_uploader?: ImageUploaderSettings;
  // Ctrl+Enter sends the message AND keeps it in the input box. Plain Enter
  // still sends + clears like normal.
  quick_send?: boolean;
  // Emote tab completion in the chat input. Tab cycles forward through
  // matching emotes/chatters, Shift+Tab cycles back.
  emote_tab_complete_enabled?: boolean;
  emote_tab_complete_match_mode?: 'starts_with' | 'includes';
  emote_tab_complete_include_chatters?: boolean;
  // Underline misspelled words in the composer and offer corrections on
  // right-click. Replaces the webview's own spell check, which flags every
  // emote name and every login because it has never seen Twitch chat.
  // Defaults to on.
  spellcheck_enabled?: boolean;
  // Words taught from the right-click menu. Stored lowercase.
  spellcheck_custom_words?: string[];
  // Chrome around the composer, for people who want the row as bare as
  // possible. All default to showing.
  hide_placeholder?: boolean;
  hide_emote_button?: boolean;
  // Hide the command menu button (the slash-in-a-box left of the emote
  // button). Typing / still opens the autocomplete.
  hide_command_button?: boolean;
  hide_points_balance?: boolean;
}

// Screen anchor a toast popup appears at. Mirrors the Rust `toast_position`
// string field; keep the two in sync.
export type ToastPosition =
  | 'top-left'
  | 'top-center'
  | 'top-right'
  | 'bottom-left'
  | 'bottom-center'
  | 'bottom-right';

export const DEFAULT_TOAST_POSITION: ToastPosition = 'bottom-right';
// Default gap from the anchored top/bottom edge, in px. Lifts toasts clear of
// the chat input (which sits in the bottom-right corner) out of the box.
export const DEFAULT_TOAST_EDGE_OFFSET = 72;

/** Identity for a favorited channel, so an unfollowed favorite can still be
 *  drawn while it is offline. Mirrors the Rust `FavoriteChannel`. */
export interface FavoriteChannel {
  /** The key used in favorite_streamers: a Twitch numeric user id, or a
   *  composite `provider:channel`. See utils/favorites. */
  id: string;
  provider: ProviderId;
  /** Login / slug / @handle / UC id: what chat and playback address. */
  channel: string;
  display_name?: string;
  avatar?: string;
  added_at?: string;
}

export interface LiveNotificationSettings {
  enabled: boolean;
  play_sound: boolean;
  sound_type?: string; // 'boop' | 'tick' | 'gentle' | 'soft' | 'whisper'
  // Notification type toggles
  show_live_notifications?: boolean;
  // Go-live notifications for FAVORITED channels, which may not be followed.
  // Separate from show_live_notifications so a long favorites list can be
  // silenced without losing notifications for the channels you follow.
  show_favorite_live_notifications?: boolean;
  show_whisper_notifications?: boolean;
  show_update_notifications?: boolean;
  show_drops_notifications?: boolean;
  show_favorite_drops_notifications?: boolean; // Notify on startup when favorited categories have new drops
  show_channel_points_notifications?: boolean;
  show_badge_notifications?: boolean;
  // Notification method toggles (Dynamic Island vs Toast)
  use_dynamic_island?: boolean;
  use_toast?: boolean;
  // Native OS notifications (Windows/macOS)
  use_native_notifications?: boolean;
  native_only_when_unfocused?: boolean;
  // Quick update: clicking update toast immediately starts update
  quick_update_on_toast?: boolean;
  // Toast placement: anchor + distance from the anchored top/bottom edge (px)
  toast_position?: ToastPosition;
  toast_edge_offset?: number;
  // Android background delivery. The in-app path needs a live WebView, so these
  // drive the WorkManager poll that keeps notifications arriving once the app is
  // closed. Desktop never reads them.
  background_checks?: boolean;
  background_interval_minutes?: number;
  // Android instant push (FCM) registration. Desktop never reads it.
  push_notifications?: boolean;
  // Channel logins excluded from live alerts. Empty means everything you follow
  // notifies, so an upgrade changes nothing until a channel is opted out.
  muted_live_channels?: string[];
}

export type AutoSwitchMode = 'same_category' | 'followed_streams';

export interface AutoSwitchSettings {
  enabled: boolean;
  mode: AutoSwitchMode;         // 'same_category' = switch to stream in same game, 'followed_streams' = switch to a followed streamer
  show_notification: boolean;   // Show toast when auto-switching
  auto_redirect_on_raid?: boolean; // Automatically follow raids to the target channel
  stay_in_offline_chat?: boolean;  // Do not auto-switch if stream goes offline, instead fallback to offline chat mode
}

// Compact View Presets
export interface CompactViewPreset {
  id: string;              // Unique identifier (e.g., 'preset-1080x608' or 'custom-1')
  name: string;            // Display name (e.g., "1080p Second Monitor")
  width: number;           // Window width in pixels
  height: number;          // Window height in pixels (excluding title bar)
  isBuiltIn: boolean;      // true for built-in presets, false for custom
}

export interface CompactViewSettings {
  selectedPresetId: string;           // Currently selected preset ID
  customPresets: CompactViewPreset[]; // User-defined custom presets
}

// Custom Theme Types
export interface CustomThemeColor {
  value: string;      // hex (#rrggbb) format
  opacity: number;    // 0-100 for colors that support opacity
}

export interface CustomThemePalette {
  // Core colors
  background: CustomThemeColor;
  backgroundSecondary: CustomThemeColor;
  backgroundTertiary: CustomThemeColor;
  
  // Surface colors
  surface: CustomThemeColor;
  surfaceHover: CustomThemeColor;
  surfaceActive: CustomThemeColor;
  
  // Text colors
  textPrimary: CustomThemeColor;
  textSecondary: CustomThemeColor;
  textMuted: CustomThemeColor;
  
  // Accent colors
  accent: CustomThemeColor;
  accentHover: CustomThemeColor;
  accentMuted: CustomThemeColor;
  
  // Border colors
  border: CustomThemeColor;
  borderLight: CustomThemeColor;
  borderSubtle: CustomThemeColor;
  
  // Semantic colors
  success: CustomThemeColor;
  warning: CustomThemeColor;
  error: CustomThemeColor;
  info: CustomThemeColor;
  
  // Special colors
  scrollbarThumb: CustomThemeColor;
  scrollbarTrack: CustomThemeColor;
  
  // Glass effect opacities (stored as decimal string, e.g., "0.15")
  glassOpacity: string;
  glassHoverOpacity: string;
  glassActiveOpacity: string;
  
  // Highlight colors
  highlight: {
    pink: CustomThemeColor;
    purple: CustomThemeColor;
    blue: CustomThemeColor;
    cyan: CustomThemeColor;
    green: CustomThemeColor;
    yellow: CustomThemeColor;
    orange: CustomThemeColor;
    red: CustomThemeColor;
  };
}

export interface CustomTheme {
  id: string;         // unique identifier (e.g., 'custom-1704567890')
  name: string;       // user-defined name
  createdAt: number;  // timestamp
  updatedAt: number;  // timestamp
  palette: CustomThemePalette;
}

export interface MultiNookSlot {
  id: string;             // Unique identifier for the slot (e.g., cell-1)
  // The platform this tile is on. ABSENT MEANS TWITCH, matching the bare-key
  // convention in utils/providerKey.ts, so grids saved before this field keep
  // loading untouched. The Rust MultiNookSlot must name this field too: that
  // struct is typed on Settings and never reaches the `extra` catch-all, so a
  // key it does not know is dropped on save (see its `quality` comment).
  provider?: ProviderId;
  channelLogin: string;   // The channel's login/slug on `provider`
  channelId?: string;     // The platform user ID for chat connection mapping
  channelName?: string;   // The capitalization-correct display name
  volume: number;         // 0.0 to 1.0
  muted: boolean;         // Mute state
  isFocused: boolean;     // Whether this stream has focus (unmuted while all others are muted)
  streamUrl?: string;     // The local proxy URL (ephemeral, not saved)
  isMinimized?: boolean;  // Whether the stream is tucked into the tray to save grid space
  profileImageUrl?: string; // Cache of the channel's profile avatar
  gameName?: string;         // Current stream category (for rich presence majority game)
  quality?: string;          // Preferred Streamlink quality for this tile (defaults to 'best')
  loadError?: boolean;       // Ephemeral: the proxy failed to start (offline/unreachable). Not persisted.
  title?: string;            // Ephemeral: current stream title, refreshed from Helix while the grid is open. Not persisted, since a saved title goes stale the moment the streamer edits it.
  broadcasterType?: string;  // Ephemeral: 'partner' | 'affiliate' | ''. Drives the verified mark on the tile. Resolved from helix/users, which every slot-creating path already calls.
}

/** A single channel stored inside a MultiNook preset. Deliberately a lean subset
 *  of MultiNookSlot. A preset records *which* channels to open, not transient
 *  view state (volume/mute/focus/minimize), which is re-derived on load. */
export interface MultiNookPresetChannel {
  provider?: ProviderId;     // Platform; absent means Twitch, as on MultiNookSlot
  channelLogin: string;      // The channel's login/slug on `provider` (canonical key)
  channelId?: string;        // Platform user ID, cached for instant chat mapping on load
  channelName?: string;      // Capitalization-correct display name for the preset UI
  profileImageUrl?: string;  // Cached avatar so preset rows render without a network hit
  quality?: string;          // Preferred Streamlink quality carried into the loaded tile
}

/** Visual icon for a MultiNook preset: either a Twitch game category's box art or
 *  one of the preset's channel avatars. Absent means fall back to the default
 *  overlapping avatar stack built from the preset's channels. */
export interface MultiNookPresetIcon {
  type: 'game' | 'channel';
  imageUrl: string;  // Ready-to-render image (box art at a fixed size, or an avatar)
  label?: string;    // Game name or channel display name, used for alt text
}

/** A named, reusable set of channels the user can open into the MultiNook grid in
 *  one click (e.g. "FNCS", "ALGS"). Presets are pure data: they hold no live
 *  proxies or players, so unloaded presets consume nothing at runtime. */
export interface MultiNookPreset {
  id: string;                       // Stable unique id (e.g. preset-<timestamp>)
  name: string;                     // User-facing label
  channels: MultiNookPresetChannel[];
  icon?: MultiNookPresetIcon;       // Optional custom icon; default is the avatar stack
  createdAt: number;                // Epoch ms, for stable default ordering
  updatedAt: number;                // Epoch ms, bumped on every edit
}

/** Customizable keyboard shortcut overrides. Maps a bindable-command id to its
 *  user-assigned chord strings (e.g. { 'player.mute': ['M', 'Ctrl+M'] }). Absent
 *  ids fall back to the code-defined defaults; an explicit empty array means the
 *  user intentionally cleared all binds for that command. */
export type KeybindingOverrides = Record<string, string[]>;

// Animation level for the whole UI (Interface > Motion). 'full' animates
// everything; 'reduced' keeps quick fades but drops movement; 'off' is instant.
export type MotionMode = 'full' | 'reduced' | 'off';

// Which of the two floating chat cards sits on top when both are live. Twitch
// allows a poll and a prediction to run at once, so both stack either way.
export type ChatOverlayOrder = 'prediction-first' | 'poll-first';

// How strictly two messages must match to count as a repeat. 'normalized' also
// folds case, spacing and trailing punctuation, so "LULW!!" joins "lulw".
export type RepeatMatchMode = 'exact' | 'normalized';

// What to do with a run of identical messages. 'collapse' keeps the first and
// folds the rest into its counter; 'label' leaves every message in place and
// just numbers them; 'off' disables detection entirely.
export type RepeatDisplayMode = 'collapse' | 'label' | 'off';

// Folding runs of the same message (a copypasta wave, or 30 people posting one
// emote) into a single row with a count. Matching is cross-user.
export interface MessageRepeatSettings {
  mode?: RepeatDisplayMode;
  match?: RepeatMatchMode;
  // Copies needed before the counter appears. 2 shows "x2" on the first repeat.
  threshold?: number;
  // How long after the first message a copy still joins its run, in seconds.
  window_seconds?: number;
  // Counter colour. Empty falls back to the muted chat text colour.
  color?: string;
  // Never fold messages from the broadcaster, moderators or VIPs.
  exempt_privileged?: boolean;
  // Keep every message visible in channels where you're a moderator, so a
  // hidden copy can never be a message you needed to action.
  keep_all_when_moderator?: boolean;
}

// Hiding chat from chosen users and known bots. Names match the stream
// overlay's blocklist dialect: case-insensitive, leading @ stripped, matched
// against login OR display name. Per-channel keys are composite provider keys
// (`makeKey`, e.g. `twitch:xqc`); utils/chatFilters.ts owns the matching.
/** One command pattern to hide: "!" as a prefix hides every !command; "!drops"
 *  as exact hides only that word at the start of a message. */
export interface CommandFilter {
  value: string;
  mode: 'prefix' | 'exact';
}

export type ChatEventCategory = 'subscription' | 'gift' | 'raid' | 'cheer' | 'milestone' | 'follow' | 'announcement';

export interface ChatEventSettings {
  // 'cards' (default): the tinted event cards. 'outline': a ring in the event
  // color on a plain row. 'plain': the row with no decoration.
  event_style?: 'cards' | 'outline' | 'plain';
  // Ring color for 'outline'; empty follows the theme accent.
  event_outline_color?: string;
  // A one-shot glint on each event row: none (default), sheen, pulse, chase.
  event_animation?: 'none' | 'sheen' | 'pulse' | 'chase';
  // Keep the glint cycling instead of playing once.
  event_animate_repeat?: boolean;
  // A Twitch cheer as its own card (default) or as an ordinary message.
  cheer_display?: 'card' | 'message';
  // Show Super Chat amounts converted to this currency code; empty = as sent.
  superchat_currency?: string;
  // Event categories to leave out, keyed "provider:category" (e.g. "twitch:gift").
  hidden_provider_events?: string[];
  // Custom wording per category with {tokens}; a template that names a token
  // the event lacks is skipped in favour of the platform's wording.
  event_templates?: Partial<Record<ChatEventCategory, string>>;
}

export interface ChatFilterSettings {
  // Hide well-known bots (StreamElements, Nightbot, ...) in every channel.
  hide_bots?: boolean;
  // Hidden everywhere, on every platform.
  hidden_users?: string[];
  // Hidden only in one channel: composite channel key -> names.
  per_channel?: Record<string, string[]>;
  // Phrases that hide a message everywhere. Evaluated in Rust
  // (services/chat_rules.rs) before the message reaches any window.
  ignored_phrases?: IgnoredPhrase[];
  // Hide messages that are bot commands (start with a pattern below). Default off.
  hide_commands?: boolean;
  // The command patterns; defaults to a single "!" prefix when empty.
  command_filters?: CommandFilter[];
}

export interface IgnoredPhrase {
  id: string;
  pattern: string;
  enabled: boolean;
  case_sensitive?: boolean;
  whole_word?: boolean;
  is_regex?: boolean;
}

// A saved message filter in Chatterino's expression syntax, evaluated in
// Rust for every message. Panes bind to a filter id and show only rows the
// engine stamped with it (metadata.filter_ids).
export interface SavedChatFilter {
  id: string;
  name: string;
  expr: string;
  enabled: boolean;
}

// Rust-owned chat query engine settings: saved filters and the per-channel
// search history cap (default 1000, range 200-5000).
export interface ChatQuerySettings {
  filters?: SavedChatFilter[];
  history_cap?: number;
}

// Which rows the chat user card shows. Everything defaults to on, so a user
// who never opens this sees the card exactly as it has always looked.
export interface UserCardSettings {
  show_join_date?: boolean;
  show_followage?: boolean;
  show_follows_count?: boolean;
  show_past_subscriber?: boolean;
  show_last_live?: boolean;
  show_chatter_count?: boolean;
  // Append "(6y ago)" next to the join date, followage and sub tenure.
  show_relative_time?: boolean;
  // Link out to the user's 7TV profile from the card header.
  show_seventv_link?: boolean;
  // Pronouns from pronouns.alejo.io (one small request per unique user,
  // cached six hours in the backend). Off by default: it is a third-party call.
  show_pronouns?: boolean;
  // Private notes on a user, kept by user id in app data.
  show_notes?: boolean;
}

// What the main window's close button does. Mirrors the Rust CloseToTrayMode
// enum (kebab-case); keep the two in sync. 'with-popouts' is the default and
// the long-standing behavior.
export type CloseToTrayMode = 'with-popouts' | 'always' | 'never';

// Which of YouTube's two live-chat views to read. Mirrors the Rust
// YouTubeChatView enum (kebab-case); keep the two in sync. 'live' is the
// unfiltered firehose and the default; 'top' is YouTube's own filtered view,
// which drops messages it judges low quality so a very fast chat stays readable.
export type YouTubeChatView = 'live' | 'top';

export interface StreamerModeSettings {
  // 'auto' watches for OBS / Streamlabs / XSplit / Twitch Studio / vMix.
  mode?: 'off' | 'on' | 'auto';
}

export interface ChatOverlaySettings {
  opacity?: number; // 10-100, default 70
  width?: number;
  height?: number;
}

export interface FullscreenChatSettings {
  // 'overlay' (default) shows chat over fullscreen video; 'hidden' keeps the
  // pre-existing behaviour (video covers everything).
  mode?: 'overlay' | 'hidden';
  // Background opacity of the overlay column, 0-100. Default 55.
  opacity?: number;
  // Column width in px, 240-640. Default 340.
  width?: number;
  // Fade the column out with the player controls; hover or focus keeps it.
  // Default true.
  auto_hide?: boolean;
  // Which edge; 'auto' follows chat_placement (bottom docks to the right).
  side?: 'auto' | 'left' | 'right';
  // Phone only. In landscape the chat toggle either floats chat over the video
  // (sharing opacity, width and side with the desktop overlay) or gives it a
  // column beside the picture. Default 'overlay'.
  phone_layout?: 'overlay' | 'beside';
}

export interface Settings {
  quality: string;
  chat_placement: string;
  // When chat_placement is 'left' or 'right', hide the docked chat and reveal it
  // on hover toward that edge (it slides in and the player flexes to make room).
  chat_auto_hide?: boolean;
  // Chat laid over the video while the player is fullscreen. Same WebView, no
  // second window: the docked chat panel is lifted above Plyr's fullscreen
  // layer as a translucent column.
  fullscreen_chat?: FullscreenChatSettings;
  // Transparent always-on-top chat overlay window (#/chat-overlay). Its
  // opacity and last size; the window itself is opened by the user.
  chat_overlay?: ChatOverlaySettings;
  // Streamer mode: hide viewer counts, link previews, restricted users' rows
  // and mute highlight sounds while broadcasting. Detection runs in Rust.
  streamer_mode?: StreamerModeSettings;
  accounts: string[];
  current_account: string;
  hide_search_bar_on_startup: boolean;
  discord_rpc_enabled: boolean;
  video_player: VideoPlayerSettings;
  cache: CacheSettings;
  streamlink?: StreamlinkSettings;
  drops: DropsSettings;
  favorite_streamers: string[];
  // Display identity for the entries in favorite_streamers. A sidecar, not a
  // replacement: favorite_streamers stays the answer to "is this favorited".
  favorite_channels?: FavoriteChannel[];
  chat_design?: ChatDesignSettings;
  chat_highlights?: ChatHighlightSettings;
  chat_customization?: ChatCustomizationSettings;
  chat_commands?: ChatCommandsSettings;
  reminders?: RemindersSettings;
  chat_input?: ChatInputSettings;
  chat_render?: ChatRenderSettings;
  cosmetics?: CosmeticsSettings;
  live_notifications?: LiveNotificationSettings;
  last_seen_version?: string;
  auto_switch?: AutoSwitchSettings;
  theme?: string; // Theme ID (e.g., 'winters-glass', 'dracula', 'nord')
  font?: string; // Interface font id (see FONT_OPTIONS in themes). Default 'satoshi'.
  font_custom?: string; // Typed family name used when font === 'custom'. Ignored otherwise.
  error_reporting_enabled?: boolean; // Local diagnostic log verbosity; nothing is sent off-device (default: true)
  setup_complete?: boolean; // Whether the first-time setup wizard has been completed
  auto_claim_points_watching?: boolean; // Auto-claim the bonus chest on the channel you're actively watching. On by default; when off, a clickable chest appears on the points button. Scoped to the watched channel only (background automation is a separate opt-in plugin).
  compact_view?: CompactViewSettings; // Compact view preset settings
  custom_themes?: CustomTheme[]; // User-created custom themes
  glass_transparency?: number; // Global glassiness, 0-100 (100 = full frosted glass, 0 = solid panels). Default 100.
  oled_accent?: string; // Accent hex (#rrggbb) for the OLED theme, which lets you pick any accent. Default DEFAULT_OLED_ACCENT.
  multi_nook_slots?: MultiNookSlot[]; // Persisted multi-nook grid configurations
  multi_nook_chat_hidden?: boolean; // Whether the chat panel is globally hidden in MultiNook
  multi_nook_presets?: MultiNookPreset[]; // Saved, named channel sets openable into the grid in one click
  multi_nook_active_preset_id?: string; // Id of the preset currently loaded into the grid (the "equipped" preset), if any
  show_mod_logs?: boolean; // Whether to display the Mod Logs pane
  show_polls?: boolean; // Show the live poll overlay card at the top of chat (default on)
  show_predictions?: boolean; // Show the live prediction overlay card at the top of chat (default on)
  // Which card sits on top when a poll and a prediction are live at the same
  // time. Both always render (they stack); this only picks the order.
  chat_overlay_order?: ChatOverlayOrder;
  show_channel_point_redemptions?: boolean; // Show no-input channel-point redemptions as chat rows (default on)
  collapse_gift_subs?: boolean; // Collapse mass gift-sub bombs into one announcement row with recipients (default on)
  // How event rows (subs, gifts, bits, milestones) look and read. Frontend
  // managed; rides the Rust settings catch-all.
  chat_events?: ChatEventSettings;
  clip_chat_replay?: boolean; // Show the chat that was live during a clip, beside it in the clip player (default on)
  chat_logging?: ChatLoggingSettings; // Save chat to plain text files as you watch
  moderation?: ModerationSettings;
  keybindings?: KeybindingOverrides; // Customizable keyboard shortcut overrides (id -> chords)
  // Which action buttons show in the video player's top-right overlay, by id:
  // 'follow' | 'subscribe' | 'clip' | 'clipsvods' | 'multinook' | 'refresh' | 'close'.
  // Undefined = show all (the default). Each button still respects its own
  // context gate (clip only when clippable, multinook/refresh only when live, …).
  player_overlay_buttons?: string[];
  // How the Settings UI is presented. true (or undefined) = a centered window
  // (the default). false = a full-page layout that fills the entire app, so long
  // tabs are easier to scan with less scrolling.
  compact_settings_window?: boolean;
  // Animation level for the whole UI (Interface > Motion). 'full' (or undefined)
  // animates everything; 'reduced' keeps fades but drops movement; 'off' is
  // instant. Applied app-wide by MotionScope (data-motion + framer MotionConfig).
  motion_mode?: MotionMode;
  // What closing the main window does. Read by the Rust window-event handler.
  close_to_tray?: CloseToTrayMode;
  // Float the window above other apps while Compact View is active, so the
  // small player stays visible over a browser. Scoped to Compact View on
  // purpose: a full-size window glued in front of everything is overbearing.
  // Only the frontend reads it, so it rides Rust's `extra` catch-all.
  keep_on_top_in_compact?: boolean;
  // Tint a stream's container from the colour of what is playing in it. Only
  // the frontend reads it (Rust is handed frames, it does not decide whether to
  // ask for them), so it rides Rust's `extra` catch-all exactly like
  // keep_on_top_in_compact above. Default ON, hence every read is `!== false`.
  media_glow?: boolean;
  // Immersive mode: the picture's own colour spills onto the letterbox bars
  // above and below it. Rides `extra` like its neighbours — Rust is handed
  // frames, it never decides what the page does with the answer. Off by
  // default: it is a mode you choose, not the normal look.
  immersive_glow?: boolean;
  // Which rows the chat user card shows.
  user_card?: UserCardSettings;
  // Folding runs of the same message into one row with a count.
  message_repeat?: MessageRepeatSettings;
  // Hiding chat from chosen users and known bots, per channel or everywhere.
  // Only the frontend reads it, so it rides Rust's `extra` catch-all.
  chat_filters?: ChatFilterSettings;
  chat_query?: ChatQuerySettings;
  // Last 10 polls and predictions you started, newest first, so running the
  // same one again is two clicks in the composer.
  recent_polls?: RecentPollEntry[];
  recent_predictions?: RecentPollEntry[];
  // Channels followed inside StreamNook on platforms whose own follow list we
  // can't read (Kick, TikTok). A TYPED Rust field, not an `extra` key, because
  // the backend who's-live poller reads it.
  provider_follows?: ProviderFollow[];
  // Which YouTube live-chat view to read. A TYPED Rust field, not an `extra`
  // key, because the YouTube adapter reads it when it resolves a stream.
  youtube_chat_view?: YouTubeChatView;
  // Which platform the Home tabs are filtered to. Frontend-only, rides `extra`.
  /** Which platform the app is scoped to (the rail's selection). Frontend-only,
   *  so it rides Rust's settings catch-all as a top-level key. */
  active_platform?: ProviderId | 'all';
  // Whether the sidebar merges platforms into one list or groups them into
  // collapsible per-platform sections. Frontend-only, rides `extra`.
  sidebar_provider_grouping?: 'unified' | 'grouped';
  /** Broadcast-language filter for the Discover feed and sidebar Recommended
   *  section (Helix codes, e.g. ["fr", "de"]). Empty or absent = no filter.
   *  Frontend-only, passed to the backend per-call, so it rides Rust's
   *  settings catch-all as a top-level key. */
  discovery_languages?: string[];
  /** Opt-in account personalization for the Discover feed. Off (default) =
   *  recommendations stay anonymous: no account credentials are ever sent.
   *  Frontend-only, passed to the backend per-call, rides `extra`. */
  discovery_personalized?: boolean;
}

/** A channel the user follows inside StreamNook. Mirrors the Rust struct. */
export interface ProviderFollow {
  provider: ProviderId;
  /** Slug / @handle / UC id, lowercased for Kick, verbatim for YouTube ids. */
  channel: string;
  display_name?: string;
  /** Cached platform user id, filled on the first successful live check. */
  user_id?: string;
  /** ISO-UTC. */
  added_at: string;
  /** The user subscribes to this channel on the platform. */
  subscribed?: boolean;
  /** Imported from the platform's own follow list rather than added by hand. */
  imported?: boolean;
  /** Channel avatar captured at import (and backfilled once after a sync), so
   *  the roster draws without a per-card lookup on every app start. */
  avatar?: string;
}

/** A title plus its choices, remembered so the composer can offer it again. */
export interface RecentPollEntry {
  title: string;
  options: string[];
}

export interface ModerationSettings {
  // Surface CLEARCHAT / CLEARMSG IRC events as system rows even for non-mods.
  // Mods see them by default; this lets curious viewers see "X was timed out".
  show_mod_messages?: boolean;
  // When a mod runs /clear, suppress the local visual wipe so the user's
  // backlog stays readable. Inverse of the /clearmessages user command.
  ignore_clear_chat?: boolean;
  // Per-category highlight colors for the mod log, keyed by category
  // (see utils/modLogCategories). Unset keys fall back to the category default.
  mod_log_colors?: Record<string, string>;
  // How mod-log severity is shown: a filled card with a matching same-color
  // border (default), a colored left bar, or just a colored dot.
  mod_log_highlight_style?: 'box' | 'bar' | 'dot';
  // Reason recorded against bans and timeouts issued by /nuke. Twitch shows it
  // in the channel's mod view, so a real sentence beats the "/nuke" default.
  nuke_reason?: string;
  // Reasons offered when banning or timing out from the user card or a
  // message's moderation controls. First entry is the prefilled default.
  saved_ban_reasons?: string[];
  // Timeout durations (seconds) offered on the hover dock and drag dial.
  // Default [1, 600, 3600, 86400].
  timeout_presets?: number[];
}

export interface ModLogEvent {
  id: string;
  action: string;
  timestamp: string;
  moderator_name: string;
  /** Acting moderator's id + login, when known (EventSub feed). Enables opening their profile. */
  moderator_id?: string;
  moderator_login?: string;
  target_user_name?: string;
  /** Target user's id + login, when known. Enables opening their profile. */
  target_user_id?: string;
  target_user_login?: string;
  /** The message content involved in the action (e.g. the deleted message text). */
  message?: string;
  reason?: string;
  duration?: number;
  /** How many of the target's messages were removed (recovered from chat history).
   *  Set for ban / timeout / YouTube author-removals. */
  removed_count?: number;
  details?: Record<string, unknown>;
  /** Lowercase channel login this action happened in (multi-stream routing + labeling). */
  channel?: string;
  /** Channel display name, when available. */
  channel_display?: string;
  /** Which feed produced this entry. EventSub carries the moderator's identity; IRC is anonymized but universal. */
  source?: 'eventsub' | 'irc';
}

export interface ReleaseNotes {
  version: string;
  name: string;
  body: string;
  published_at: string;
}

/** Channel points as the Rust channel-state service reports them. */
export interface ChannelPoints {
  enabled: boolean;
  balance: number | null;
  name: string | null;
  icon_url: string | null;
  available_claim_id: string | null;
}

/** Per-channel chat state owned by Rust (src-tauri/src/services/channel_state.rs). */
export interface ChannelState {
  login: string;
  channel_id: string;
  viewer_count: number | null;
  viewers_at: number | null;
  points: ChannelPoints | null;
  points_at: number | null;
  pinned: unknown[];
  pinned_at: number | null;
}

/** One changed section, the payload of the `channel-state` event. */
export type ChannelStateUpdate =
  | { section: 'viewers'; login: string; viewer_count: number | null; at: number }
  | { section: 'points'; login: string; points: ChannelPoints | null; at: number }
  | { section: 'pinned'; login: string; pinned: unknown[]; at: number };

/** One channel's bulk hype-train status, as Rust reports it. */
export interface HypeTrainBulkStatus {
  channel_id: string;
  is_active: boolean;
  level: number;
  is_golden_kappa: boolean;
}

/** The Rust-owned Home snapshot (src-tauri/src/services/home_snapshot.rs):
 *  everything the Home grid and the Sidebar render, with a fetch time per
 *  section (unix seconds, null until first fetched). */
export interface HomeSnapshot {
  followed_live: TwitchStream[];
  followed_live_at: number | null;
  offline_follows: TwitchStream[];
  last_broadcasts: Record<string, string | null>;
  offline_at: number | null;
  recommended: TwitchStream[];
  recommended_cursor: string | null;
  recommended_at: number | null;
  hype_trains: HypeTrainBulkStatus[];
  hype_at: number | null;
  watch_streaks: Record<string, number>;
  streaks_at: number | null;
  drops_campaigns: DropCampaign[];
  drops_active_game_names: string[];
  drops_at: number | null;
  continue_watching: ContinueWatchingItem[];
  continue_watching_at: number | null;
}

/** One card in Home's Continue Watching row. Rust builds these from the local
 *  watch-position store, so the row renders with no Twitch call. Only VODs the
 *  viewer deliberately opened ever appear — live sessions, live rewinds and
 *  the offline auto-play all record nothing. */
export interface ContinueWatchingItem {
  video_id: string;
  channel_login: string;
  /** Display name, already falling back to the login in Rust. */
  channel_name: string;
  title: string;
  thumbnail_url: string;
  position_secs: number;
  duration_secs: number;
  /** Empty until Rust's hourly hydrate has seen the VOD once. */
  profile_image_url: string;
  /** Twitch partner: the card shows the verified mark. */
  partner: boolean;
  /** Category of the broadcast; empty until Rust's hydrate has seen it. */
  game_name: string;
}

/** One changed section, the payload of the `home-snapshot` event. */
export type HomeSnapshotUpdate =
  | { section: 'followed_live'; streams: TwitchStream[]; at: number }
  | { section: 'offline'; channels: TwitchStream[]; last_broadcasts: Record<string, string | null>; at: number }
  | { section: 'recommended'; streams: TwitchStream[]; cursor: string | null; at: number }
  | { section: 'hype_trains'; statuses: HypeTrainBulkStatus[]; at: number }
  | { section: 'watch_streaks'; streaks: Record<string, number>; at: number }
  | { section: 'drops'; campaigns: DropCampaign[]; active_game_names: string[]; at: number }
  | { section: 'continue_watching'; items: ContinueWatchingItem[]; at: number };

/**
 * A live stream row. The name is historical: rows may now come from any
 * platform, tagged by `provider`. Non-Twitch rows are field-compatible but
 * differ semantically — `user_id` is the platform's own id (Kick numeric,
 * YouTube UC…), `thumbnail_url` is a direct URL with no {width} template, and
 * `game_id` / `tags` / `broadcaster_type` may be absent.
 */
export interface TwitchStream {
  id: string;
  user_id: string;
  user_name: string;
  user_login: string;
  title: string;
  viewer_count: number;
  game_id?: string;
  game_name: string;
  thumbnail_url: string;
  started_at: string;
  broadcaster_type?: string;
  has_shared_chat?: boolean;
  profile_image_url?: string;
  is_live?: boolean;
  // Free-form stream tags (e.g. "English", "Speedrun"); used by the category tag filter.
  tags?: string[];
  /** Broadcast language in Helix form ("en", "fr", "zh-hk"). */
  language?: string;
  /** Source platform. ABSENT = twitch (every pre-existing producer). */
  provider?: ProviderId;
  /** Canonical platform watch URL, set by provider rows; drives `start_stream`. */
  watch_url?: string;
}

/** Platform-neutral alias for new code; identical to `TwitchStream`. */
export type Stream = TwitchStream;

export interface TwitchClip {
  id: string;
  url: string;
  embed_url: string;
  broadcaster_id: string;
  broadcaster_name: string;
  /** Broadcaster LOGIN (lowercase), distinct from the display name. Chat replay keys a
   *  channel's third-party emote set off the login. Absent on the Helix clip path, which
   *  only carries the display name; resolve it from broadcaster_id there. */
  broadcaster_login?: string;
  creator_id: string;
  creator_name: string;
  video_id: string;
  game_id: string;
  game_name?: string;
  language: string;
  title: string;
  view_count: number;
  created_at: string;
  thumbnail_url: string;
  duration: number;
  vod_offset?: number;
}

export interface TwitchVideo {
  id: string;
  stream_id?: string;
  user_id: string;
  user_login: string;
  user_name: string;
  title: string;
  description: string;
  created_at: string;
  published_at: string;
  url: string;
  thumbnail_url: string;
  viewable: string;
  view_count: number;
  language: string;
  type: string;
  duration: string;
  /** "recording" while the broadcast is still live, "recorded" when finished.
   *  Only the GQL user-videos path fills it. */
  status?: string;
  /** Length in whole seconds (the number `duration` is formatted from). */
  length_seconds?: number;
  /** The viewer's stored watch position, joined on by Rust. Absent when never
   *  watched. */
  progress?: VodProgressSummary;
  /** Category of the broadcast. Only the GQL user-videos path fills it. */
  game_name?: string;
}

/** Rust-owned VOD watch state, the slice a card needs for its bar. */
export interface VodProgressSummary {
  position_secs: number;
  duration_secs: number;
  completed: boolean;
}

/** What `start_stream` returns alongside the proxy URL for a VOD: how to run
 *  it (a "recording" VOD is a growing EVENT playlist, not live) and where to
 *  begin. */
export interface VodStartInfo {
  video_id: string;
  status: 'recording' | 'recorded' | 'unknown' | string;
  length_seconds?: number;
  recorded_at?: string;
  channel_login?: string;
  title?: string;
  thumbnail_url?: string;
  /** Resume position, or the mapped broadcast position for a live rewind. */
  start_position_secs?: number;
  /** The viewer rewound a live broadcast into this recording. */
  rewound_from_live: boolean;
  /** Audio ranges Twitch muted, merged and clamped by Rust. Absent or empty
   *  when none are known: on a `recording` VOD that means Twitch has not
   *  determined them yet, NOT that there are none. */
  muted_segments?: MutedRange[];
}

/** An audio range Twitch muted on a VOD, in seconds from the VOD start.
 *  Merged, sorted and clamped in Rust (services/muted_segments.rs). */
export interface MutedRange {
  start_secs: number;
  end_secs: number;
}

/** Rust's answer to "can this live broadcast be rewound": Twitch keeps a
 *  recording only while the channel has VODs enabled. */
export interface LiveRewindInfo {
  available: boolean;
  video_id?: string;
  recorded_at?: string;
}

export interface TwitchUser {
  access_token: string;
  username: string;
  user_id: string;
  login?: string;
  display_name?: string;
  profile_image_url?: string;
  broadcaster_type?: string;
}

export interface UserInfo {
  id: string;
  login: string;
  display_name: string;
  email?: string;
  profile_image_url?: string;
  broadcaster_type?: string;
}

export interface SevenTVBadge {
  id: string;
  name: string;
  tooltip: string;
  urls: Array<[string, string]>;
}

export interface SevenTVPaint {
  id: string;
  name: string;
  function: 'LINEAR_GRADIENT' | 'RADIAL_GRADIENT' | 'URL';
  color?: number;
  stops?: Array<{ at: number; color: number }>;
  repeat?: boolean;
  angle?: number;
  image_url?: string;
  shadows?: Array<{
    x_offset: number;
    y_offset: number;
    radius: number;
    color: number;
  }>;
}

export interface ChatUser {
  id: string;
  username: string;
  displayName: string;
  color?: string;
  badges?: Record<string, string>;
  seventvBadge?: SevenTVBadge;
  seventvPaint?: SevenTVPaint;
}

export interface TwitchGame {
  id: string;
  name: string;
  box_art_url: string;
  igdb_id?: string;
}

export interface TwitchCategory extends TwitchGame {
  viewer_count?: number;
  tags?: string[];
}

export interface CategoryTag {
  id: string;
  localizedName: string;
}

export interface CategoryInfo {
  id: string;
  name: string;
  displayName: string | null;
  description: string | null;
  followersCount: number | null;
  boxArtUrl: string | null;
  tags: CategoryTag[] | null;
}

// Unified Game Interface for Drops UI Overhaul
export interface UnifiedGame {
  id: string;                          // Game ID
  name: string;                        // Game Name
  box_art_url: string;                 // Game artwork

  // Active campaign data
  active_campaigns: DropCampaign[];    // Currently running campaigns
  total_active_drops: number;          // Sum of drops across campaigns
  drops_in_progress: number;           // Drops being worked on

  // Inventory data
  inventory_items: InventoryItem[];    // Past/earned campaigns
  total_claimed: number;               // Total claimed drops for this game

  // Status
  active: boolean;                  // Currently automation this game
  has_claimable: boolean;              // Has drops ready to claim
  all_drops_claimed: boolean;          // All available drops have been claimed (game complete)
}

// Drops inventory types
export interface DropBenefit {
  id: string;
  name: string;
  image_url: string;
  /** Distribution type - "BADGE" for Twitch chat badges, "DIRECT_ENTITLEMENT" for in-game items */
  distribution_type?: string;
}

export interface DropProgress {
  campaign_id: string;
  drop_id: string;
  current_minutes_watched: number;
  required_minutes_watched: number;
  is_claimed: boolean;
  last_updated: string;
  drop_instance_id?: string; // Required for claiming drops - compound ID from Twitch
  drop_name?: string; // Cached drop name from backend events
  drop_image?: string; // Cached drop image from backend events
}

export interface TimeBasedDrop {
  id: string;
  name: string;
  required_minutes_watched: number;
  benefit_edges: DropBenefit[];
  progress?: DropProgress;
  /** Whether this drop can be auto-collected. Drops with required_minutes_watched = 0 
   * are event-based, badge-based, or require special actions and cannot be auto-collected */
  is_collectible?: boolean;
}

export interface AllowedChannel {
  id: string;
  name: string;
}

export interface DropCampaign {
  id: string;
  name: string;
  game_id: string;
  game_name: string;
  description: string;
  image_url: string;
  start_at: string;
  end_at: string;
  time_based_drops: TimeBasedDrop[];
  is_account_connected: boolean;
  allowed_channels: AllowedChannel[];
  is_acl_based: boolean;
  account_link?: string; // URL to connect game account for drops
}

export type CampaignStatus = 'Active' | 'Upcoming' | 'Expired';

export interface InventoryItem {
  campaign: DropCampaign;
  status: CampaignStatus;
  progress_percentage: number;
  total_drops: number;
  claimed_drops: number;
  drops_in_progress: number;
}

export interface InventoryResponse {
  items: InventoryItem[];
  total_campaigns: number;
  active_campaigns: number;
  upcoming_campaigns: number;
  expired_campaigns: number;
  completed_drops: CompletedDrop[];
}

export interface CompletedDrop {
  id: string;
  name: string;
  image_url: string;
  game_name: string | null;
  is_connected: boolean;
  required_account_link: string | null;
  last_awarded_at: string;
  total_count: number;
}

export interface ClaimedDrop {
  id: string;
  campaign_id: string;
  drop_id: string;
  drop_name: string;
  game_name: string;
  benefit_name: string;
  benefit_image_url: string;
  claimed_at: string;
}

export interface ChannelPointsClaim {
  id: string;
  channel_id: string;
  channel_name: string;
  points_earned: number;
  claimed_at: string;
  claim_type: 'Watch' | 'Raid' | 'Prediction' | 'Bonus' | 'Other';
}

export interface ChannelPointsBalance {
  channel_id: string;
  channel_name: string;
  balance: number;
  last_updated: string;
  /** Custom channel points name (e.g., "Kisses" for Hamlinz). undefined = default */
  points_name?: string;
  /** Custom channel points icon URL. undefined = uses default Twitch icon */
  points_icon_url?: string;
}

/** A custom channel reward that users can redeem with channel points */
export interface ChannelReward {
  id: string;
  title: string;
  cost: number;
  prompt?: string;
  image_url?: string;
  background_color: string;
  is_enabled: boolean;
  is_paused: boolean;
  is_in_stock: boolean;
  is_user_input_required: boolean;
  cooldown_expires_at?: string;
  max_per_stream?: number;
  max_per_user_per_stream?: number;
  global_cooldown_seconds?: number;
}

/** Result of attempting to redeem a channel reward */
export interface RedemptionResult {
  success: boolean;
  error_code?: string;
  error_message?: string;
  new_balance?: number;
  /** For emote unlock rewards - contains info about the unlocked emote */
  unlocked_emote?: UnlockedEmote;
}

/** Information about an unlocked emote (for random emote unlock rewards) */
export interface UnlockedEmote {
  id: string;
  name: string;
  image_url: string;
}

export interface DropsStatistics {
  total_drops_claimed: number;
  total_channel_points_earned: number;
  active_campaigns: number;
  drops_in_progress: number;
  recent_claims: ClaimedDrop[];
  channel_points_history: ChannelPointsClaim[];
}

// Dynamic Island Notification Types
export type NotificationType = 'live' | 'whisper' | 'system' | 'update' | 'drops' | 'channel_points' | 'badge';

export interface DynamicIslandNotification {
  id: string;
  type: NotificationType;
  timestamp: number;
  read: boolean;
  data: LiveNotificationData | WhisperNotificationData | SystemNotificationData | UpdateNotificationData | DropsNotificationData | ChannelPointsNotificationData | BadgeNotificationData;
}

export interface LiveNotificationData {
  streamer_name: string;
  streamer_login: string;
  /** Every platform this one entry stands for.
   *
   *  A channel that goes live on Twitch and then on Kick is one event to the
   *  person reading it, so the two collapse into a single entry carrying both
   *  marks rather than two rows with the same face and name. `streamer_login`
   *  stays whatever the FIRST one carried, so the click still opens something
   *  real. Optional because entries cached before this existed have no idea. */
  providers?: ProviderId[];
  streamer_avatar?: string;
  game_name?: string;
  game_image?: string;
  stream_title?: string;
  is_live: boolean; // Current status - may change
  is_test?: boolean; // Dummy preview entry from the "Test" button; clicking it must not open a real stream
}

export interface WhisperNotificationData {
  from_user_id: string;
  from_user_login: string;
  from_user_name: string;
  message: string;
  whisper_id: string;
  profile_image_url?: string;
}

export interface SystemNotificationData {
  /** Where the entry came from, when that is more useful than its level.
   *  A plugin saying "farming is now active" is a plugin talking, and the
   *  generic info glyph buries that under the same mark every other status
   *  message gets. */
  source?: 'plugin';
  /** A face for entries that are ABOUT somebody, in practice the signed-in user.
   *  "Welcome back, br_winters" with a generic success tick says the operation
   *  succeeded, which is not the point of it; the point is who you are. When
   *  absent the row falls back to the level glyph, which is right for the ones
   *  that really are just status. */
  avatar_url?: string;
  title: string;
  message: string;
  icon?: string;
  // Toast level this entry was mirrored from, so the island can pick an icon/color.
  level?: 'info' | 'success' | 'warning' | 'error';
}

export interface UpdateNotificationData {
  current_version: string;
  latest_version: string;
  has_update: boolean;
}

export interface DropsNotificationData {
  drop_name: string;
  game_name: string;
  benefit_name?: string;
  benefit_image_url?: string;
}

export interface ChannelPointsNotificationData {
  channel_name: string;
  points_earned: number;
  total_points?: number;
}

export interface BadgeNotificationData {
  badge_name: string;
  badge_set_id: string;
  badge_version: string;
  badge_image_url: string;
  badge_description?: string;
  status: 'new' | 'available' | 'coming_soon';
  date_info?: string; // e.g., "Dec 1-12" or "Available now"
  // Relay campaign facts, kept so the stored row can re-derive its status at
  // render time instead of freezing the one that was true when it arrived.
  enrichment?: Record<string, unknown>;
}

// Whisper Types
export interface Whisper {
  id: string;
  from_user_id: string;
  from_user_login: string;
  from_user_name: string;
  to_user_id: string;
  to_user_login: string;
  to_user_name: string;
  message: string;
  timestamp: number;
  is_sent: boolean; // true if sent by current user
}

export interface WhisperConversation {
  user_id: string;
  user_login: string;
  user_name: string;
  profile_image_url?: string;
  messages: Whisper[];
  last_message_timestamp: number;
  unread_count: number;
}

// Hype Train Types
export interface HypeTrainContributor {
  user_id: string;
  user_login: string;
  user_name: string;
  type: 'bits' | 'subscription' | 'other';
  total: number;
}

export interface HypeTrainData {
  id: string;
  broadcaster_user_id: string;
  broadcaster_user_login: string;
  broadcaster_user_name: string;
  level: number;
  total: number;
  progress: number;
  goal: number;
  top_contributions: HypeTrainContributor[];
  started_at: string;
  expires_at: string;
  is_golden_kappa?: boolean;
}





