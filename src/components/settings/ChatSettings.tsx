import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
// This panel is genuinely SHARED - both shells render it - so a platform branch
// here is legitimate, unlike in components that only ever run on one platform.
// It hides rows whose backing feature does not exist on Android, so the phone
// settings stop offering knobs that quietly do nothing.
import { IS_MOBILE } from '../../utils/platform';
import PanelChannelList from '../plugins/PanelChannelList';
import { trustableHost } from '../../services/linkPreviewService';
import { Tooltip } from '../ui/Tooltip';
import { Dropdown } from '../ui/Dropdown';
import HighlightPhrasesSettings from './HighlightPhrasesSettings';
import BuiltInHighlightsSettings from './BuiltInHighlightsSettings';
import UserHighlightsSettings from './UserHighlightsSettings';
import BadgeHighlightsSettings from './BadgeHighlightsSettings';
import HighlightAppearanceSettings from './HighlightAppearanceSettings';
import UserOverridesSettings from './UserOverridesSettings';
import UserCommandsSettings from './UserCommandsSettings';
import RemindersSettings from './RemindersSettings';
import { SettingsSection, SettingsRow, SegmentedSelect } from './_primitives';
import { useSettingReset } from './settingReset';
import { Toggle } from '../ui/Toggle';
import { usePhonePrefs } from '../../mobile/phonePrefs';
import { useNameColorAdjust } from '../../hooks/useNameColor';
import { CURRENCY_OPTIONS } from '../../services/currencyService';
import { EVENT_CATEGORIES, EVENT_TEMPLATE_EXAMPLES, PROVIDER_CATEGORY_LABELS, PROVIDER_EVENT_CATEGORIES } from '../overlay/overlayConfig';
import type { ChatEventCategory, ChatEventSettings, CommandFilter } from '../../types';
import SpellcheckDictionary from './SpellcheckDictionary';
import IgnoredPhrasesSettings from './IgnoredPhrasesSettings';
import CustomSoundsSettings from './CustomSoundsSettings';
import ImageUploadSettings from './ImageUploadSettings';
import SavedFiltersSettings from './SavedFiltersSettings';
import type {
  UserCardSettings,
  MessageRepeatSettings,
  RepeatDisplayMode,
  RepeatMatchMode,
  YouTubeChatView,
  ChatFilterSettings,
} from '../../types';
import { filterChannelKey } from '../../utils/chatFilters';
import { Logger } from '../../utils/logger';
import { parseKey } from '../../utils/providerKey';
import { CHAT_PROVIDERS, PROVIDERS, type ProviderId } from '../../types/providers';

/** Platforms that can join a combined feed. Derived from the chat-capability
 *  flags rather than hand-listed, so a newly chat-enabled platform appears here
 *  without a second place to remember. Twitch is excluded only as a *companion*
 *  choice when Twitch is what you are watching; the filter is per-row, not here. */
const BLEND_PLATFORMS: ProviderId[] = CHAT_PROVIDERS;

// Muted grey, so the counter reads as chrome rather than competing with the
// message. Matches --color-text-secondary in the default theme.
const REPEAT_DEFAULT_COLOR = '#8b8b8b';

// Rows the user card can show, in the order they appear on the card itself.
// Everything defaults to on; the toggle stores `false` to hide.
const USER_CARD_ROWS: { key: keyof UserCardSettings; title: string; description: string }[] = [
  { key: 'show_join_date', title: 'Join date', description: 'When their Twitch account was created.' },
  { key: 'show_followage', title: 'Following since', description: 'When they followed this channel, or that they are not following.' },
  { key: 'show_follows_count', title: 'Follow count', description: 'How many channels this person follows.' },
  { key: 'show_chatter_count', title: 'Chatters', description: "How many people are in this person's own chat right now." },
  { key: 'show_past_subscriber', title: 'Past subscriber', description: 'Total months subscribed, for people who are not subscribed now.' },
  { key: 'show_last_live', title: 'Last live', description: 'When they last streamed, if they ever have.' },
  { key: 'show_relative_time', title: 'Relative dates', description: 'Adds a plain-English age next to dates, so "Mar 3, 2019" also reads "(6y ago)".' },
  { key: 'show_seventv_link', title: '7TV profile link', description: 'A 7TV chip next to their name that opens their 7TV profile in your browser.' },
  { key: 'show_pronouns', title: 'Pronouns', description: 'Their pronouns from pronouns.alejo.io, where chatters set them once for every chat client. One small request per person, cached for six hours. Off by default because it is a third-party lookup.' },
  { key: 'show_notes', title: 'Private notes', description: 'A note only you can see, kept with the user across renames. Handy for moderators.' },
];
import { useChatUserStore } from '../../stores/chatUserStore';
import { getUserCosmetics, computePaintStyle } from '../../services/seventvService';
import { StyledChatName, type NameSeparator, type NameStyle } from '../chat/StyledChatName';
import type { DeletedMessageStyle } from '../chat/deletedMessage';
import { DeletedMessagePreview, EventRowPreview, MentionPreview, MessageLayoutPreview, SettingPreview } from './ChatPreview';
import { SoundVolume } from './SoundControls';
import { InlineSlider, SubControl, SubControls } from '../plugins/settingsPageKit';
import { useSoundOptions } from '../../hooks/useSoundOptions';
import { mentionLook, type MentionItalic, type MentionLook, type MentionShape, type MentionWeight } from '../chat/mentionStyle';

// Native color swatch matching the mod-log Log Highlights control: clicking it
// opens the OS picker (always on top, unlike an in-app popover that can render
// behind later settings rows). Its row's reset arrow restores the default.
const ColorSwatch = ({
  value,
  onChange,
  tooltip,
}: {
  value: string;
  onChange: (color: string) => void;
  tooltip: string;
}) => (
  <Tooltip content={tooltip}>
    <input
      type="color"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle"
    />
  </Tooltip>
);

// Live preview of how the current user's own name will look in chat with the
// chosen separator + name style, including their selected 7TV paint. Shares
// StyledChatName with the real chat row so the preview can never drift from it.
type PreviewPaint = Awaited<ReturnType<typeof getUserCosmetics>>['data']['paints'][number];

const NamePrefixPreview = ({
  separator,
  nameStyle,
  accentSource,
}: {
  separator: NameSeparator;
  nameStyle: NameStyle;
  accentSource: 'user' | 'theme';
}) => {
  const currentUser = useAppStore((s) => s.currentUser);
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  const adjustPreviewColor = useNameColorAdjust();
  const fontSize = useAppStore((s) => s.settings.chat_design?.font_size) ?? 14;
  const userId = currentUser?.user_id;
  const storeEntry = useChatUserStore((s) => (userId ? s.users.get(userId) : undefined));
  const [fetchedPaint, setFetchedPaint] = useState<PreviewPaint | null>(null);

  // If chat hasn't already resolved this user's cosmetics (their paint stays
  // undefined in the store until addUser runs), fetch them once so the preview
  // still shows the real paint while sitting in settings.
  useEffect(() => {
    if (!userId || storeEntry?.paint !== undefined) return;
    let cancelled = false;
    getUserCosmetics(userId)
      .then(({ data }) => {
        if (cancelled) return;
        setFetchedPaint(data?.paints?.find((p: { selected?: boolean }) => p.selected) ?? null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [userId, storeEntry?.paint]);

  const name = currentUser?.display_name || currentUser?.username || 'YourName';
  const baseColor = adjustPreviewColor(storeEntry?.color || '#9147ff') ?? '#9147ff';
  const paint = storeEntry?.paint ?? fetchedPaint;
  const nameTextStyle = paint ? computePaintStyle(paint, baseColor, paintShadowMode) : { color: baseColor };
  const accentColor = accentSource === 'theme' ? 'var(--color-accent)' : baseColor;

  return (
    <div className="glass-panel rounded-lg px-3 py-2.5" style={{ fontSize: `${fontSize}px`, lineHeight: 1.5 }}>
      <StyledChatName
        name={name}
        nameTextStyle={nameTextStyle}
        nameStyle={nameStyle}
        separator={separator}
        accentColor={accentColor}
      />
      <span className="text-textPrimary/90" style={{ fontWeight: 'var(--chat-body-weight, 300)' }}>
        {' '}gg that was clean
      </span>
    </div>
  );
};

// Discrete hover-preview sizes (px height of the enlarged card). 'Medium' is
// the default and sits one step above the original fixed 64px preview.
const HOVER_SIZE_OPTIONS = [
  { value: 'sm', label: 'Small', px: 64 },
  { value: 'md', label: 'Medium', px: 96 },
  { value: 'lg', label: 'Large', px: 128 },
  { value: 'xl', label: 'Huge', px: 160 },
] as const;

type HoverSizeKey = (typeof HOVER_SIZE_OPTIONS)[number]['value'];

// A widely-recognized 7TV emote used purely as the live sample so the preview
// renders a real emote with proper upscaling at any size.
const SAMPLE_EMOTE_ID = '01GA29CZ2R000C36HNE7Z0DQXD';
const SAMPLE_EMOTE_NAME = 'KEKW';

// Live, hoverable demo of the emote hover preview. The inline emote renders at
// the user's chosen Emote Size (emoteScale); hovering it pops the real hover
// card sized to hoverSize, so the row reflects both settings as they change.
const EmoteHoverDemo = ({ hoverSize, emoteScale }: { hoverSize: number; emoteScale: number }) => {
  const previewCard = (
    <div className="flex flex-col items-center gap-1.5 py-0.5">
      <img
        src={`https://cdn.7tv.app/emote/${SAMPLE_EMOTE_ID}/4x.avif`}
        alt={SAMPLE_EMOTE_NAME}
        className="w-auto object-contain mx-auto drop-shadow-md"
        style={{ height: hoverSize, maxWidth: hoverSize * 2 }}
        referrerPolicy="no-referrer"
      />
      <span className="font-bold text-[13px] leading-tight">{SAMPLE_EMOTE_NAME}</span>
      <span className="text-[10px] text-white/60 leading-tight">7TV</span>
    </div>
  );
  return (
    <div className="flex items-center justify-center gap-2 rounded-lg border border-white/5 bg-black/20 px-4 py-3">
      <span className="select-none text-[12px] text-textSecondary">Hover the emote</span>
      <span className="select-none text-[12px] text-textMuted">&rarr;</span>
      <Tooltip content={previewCard} side="top">
        <img
          src={`https://cdn.7tv.app/emote/${SAMPLE_EMOTE_ID}/2x.avif`}
          alt={SAMPLE_EMOTE_NAME}
          className="inline-block w-auto cursor-pointer align-middle transition-transform hover:scale-110"
          style={{ height: `calc(1.75rem * ${emoteScale})` }}
          referrerPolicy="no-referrer"
        />
      </Tooltip>
    </div>
  );
};

// Manage the user's own trusted-source list: an add input plus removable chips.
// The built-in allowlist isn't listed (it'd be noise); a short note names the
// kinds of sites that are trusted out of the box. Hosts are normalized through
// `trustableHost` so a pasted URL becomes a clean registrable host.
const TrustedSourcesEditor = ({
  domains,
  onChange,
}: {
  domains: string[];
  onChange: (next: string[]) => void;
}) => {
  const [input, setInput] = useState('');
  const pending = trustableHost(input.trim());

  const add = () => {
    if (!pending) return;
    if (!domains.includes(pending)) onChange([...domains, pending]);
    setInput('');
  };
  const remove = (host: string) => onChange(domains.filter((d) => d !== host));

  return (
    <div className="space-y-3">
      <label className="block text-[11px] text-textSecondary">Site to trust</label>
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          placeholder="example.com"
          className="glass-input min-w-0 flex-1 rounded-lg px-3 py-2 text-sm text-textPrimary placeholder:text-textMuted"
        />
        <button
          onClick={add}
          disabled={!pending}
          className="flex-shrink-0 rounded-lg bg-accent/15 px-3.5 py-2 text-sm font-medium text-accent transition-colors hover:bg-accent/25 disabled:cursor-not-allowed disabled:opacity-40"
        >
          Add
        </button>
      </div>
      {domains.length > 0 ? (
        <div className="flex flex-wrap gap-1.5">
          {domains.map((host) => (
            <span
              key={host}
              className="glass-panel inline-flex items-center gap-1.5 rounded-full py-1 pl-3 pr-1.5 text-xs text-textPrimary"
            >
              {host}
              <button
                onClick={() => remove(host)}
                aria-label={`Stop trusting ${host}`}
                className="flex h-4 w-4 items-center justify-center rounded-full text-textSecondary transition-colors hover:bg-white/10 hover:text-textPrimary"
              >
                <X size={12} />
              </button>
            </span>
          ))}
        </div>
      ) : (
        <p className="text-[12px] leading-relaxed text-textMuted">
          No custom sites trusted yet. Popular sites (YouTube, Twitch, Discord, Steam,
          Spotify, imgur, Tenor, and more) already expand by default.
        </p>
      )}
    </div>
  );
};

// `hidePlacement` drops the Chat Placement section — it positions the MAIN app's
// chat (left/right/bottom/hidden), which is meaningless in the MultiChat window's
// own settings.
// Small add/remove name list for the chat filters. Mirrors the overlay's
// blocklist editor: type a name, Enter or Add commits it; chips remove.
const HiddenNameEditor = ({
  names,
  onAdd,
  onRemove,
}: {
  names: string[];
  onAdd: (name: string) => void;
  onRemove: (name: string) => void;
}) => {
  const [val, setVal] = useState('');
  const add = () => {
    const n = val.trim();
    if (n) {
      onAdd(n);
      setVal('');
    }
  };
  return (
    <div className="flex flex-col gap-2 w-full max-w-sm">
      <label className="block text-[11px] text-textSecondary">Username to hide</label>
      <div className="flex gap-2">
        <input
          value={val}
          onChange={(e) => setVal(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') add(); }}
          placeholder="Type a username"
          className="flex-1 bg-surface border border-borderSubtle rounded px-2.5 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:outline-none focus:border-accent"
        />
        <button
          onClick={add}
          className="px-3 py-1.5 text-sm rounded bg-surface hover:bg-surface-hover text-textPrimary border border-borderSubtle"
        >
          Add
        </button>
      </div>
      {names.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {names.map((n) => (
            <span key={n} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-surface text-xs text-textPrimary">
              {n}
              <button aria-label={`Unhide ${n}`} className="text-textSecondary hover:text-error" onClick={() => onRemove(n)}>×</button>
            </span>
          ))}
        </div>
      )}
    </div>
  );
};

const PROVIDER_LABELS: Record<string, string> = { twitch: 'Twitch', kick: 'Kick', youtube: 'YouTube', tiktok: 'TikTok' };

/** Add-on badge services, by the lowercase id each resolved badge carries. */
const BADGE_PROVIDERS: { id: string; label: string }[] = [
  { id: 'streamnook', label: 'StreamNook' },
  { id: '7tv', label: '7TV' },
  { id: 'ffz', label: 'FFZ' },
  { id: 'bttv', label: 'BTTV' },
  { id: 'chatterino', label: 'Chatterino' },
  { id: 'homies', label: 'Homies' },
  { id: 'moltorino', label: 'Moltorino' },
  { id: 'chatsen', label: 'Chatsen' },
  { id: 'chatty', label: 'Chatty' },
  { id: 'dankchat', label: 'DankChat' },
];

const ChatSettings = ({ hidePlacement = false }: { hidePlacement?: boolean } = {}) => {
  const { settings, updateSettings } = useAppStore();
  const resetFor = useSettingReset();
  // Phone-shell preferences (see mobile/phonePrefs.ts); only read on the phone.
  const mentionHaptic = usePhonePrefs((s) => s.mentionHaptic);
  const setMentionHaptic = usePhonePrefs((s) => s.setMentionHaptic);
  // Chat events (how event rows look and read) and command hiding.
  const chatEvents = settings.chat_events ?? {};
  const setEvents = (patch: Partial<ChatEventSettings>) =>
    updateSettings({ ...settings, chat_events: { ...settings.chat_events, ...patch } });
  const hiddenEvents = chatEvents.hidden_provider_events ?? [];
  const toggleHiddenEvent = (key: string) =>
    setEvents({
      hidden_provider_events: hiddenEvents.includes(key) ? hiddenEvents.filter((k) => k !== key) : [...hiddenEvents, key],
    });
  // The option under the pointer in a visual setting, shown in its preview
  // before it is picked; null falls back to the saved value.
  const [deletedHover, setDeletedHover] = useState<DeletedMessageStyle | null>(null);
  const [entranceHover, setEntranceHover] = useState<'none' | 'fade' | 'slide' | 'rise' | null>(null);
  // Hovering a mention-style option previews it before it is chosen.
  const [mentionHover, setMentionHover] = useState<Partial<MentionLook>>({});
  const hoverMention = <K extends keyof MentionLook>(key: K) => (value: MentionLook[K] | null) =>
    setMentionHover((h) => {
      const next = { ...h };
      if (value === null) delete next[key];
      else next[key] = value;
      return next;
    });
  const mentionSoundOptions = useSoundOptions({ noneLabel: 'No sound' });
  const [eventStyleHover, setEventStyleHover] = useState<'cards' | 'outline' | 'plain' | null>(null);
  const [glintHover, setGlintHover] = useState<'none' | 'sheen' | 'pulse' | 'chase' | null>(null);
  const [commandDraft, setCommandDraft] = useState('');
  const [commandMode, setCommandMode] = useState<'prefix' | 'exact'>('prefix');
  const commandFilters: CommandFilter[] = settings.chat_filters?.command_filters ?? [];
  const setCommandFilters = (next: CommandFilter[], hide?: boolean) =>
    updateSettings({
      ...settings,
      chat_filters: {
        ...settings.chat_filters,
        command_filters: next,
        ...(hide === undefined ? {} : { hide_commands: hide }),
      },
    });

  const stored = settings.chat_design;
  const cd = {
    show_dividers: stored?.show_dividers ?? true,
    alternating_backgrounds: stored?.alternating_backgrounds ?? false,
    message_spacing: stored?.message_spacing ?? 8,
    font_size: stored?.font_size ?? 14,
    activity_font_size: stored?.activity_font_size ?? 14,
    font_weight: stored?.font_weight ?? 400,
    mention_color: stored?.mention_color ?? '#ff4444',
    reply_color: stored?.reply_color ?? '#ff6b6b',
    mention_animation: stored?.mention_animation ?? true,
    show_timestamps: stored?.show_timestamps ?? false,
    show_timestamp_seconds: stored?.show_timestamp_seconds ?? false,
    timestamp_format: stored?.timestamp_format ?? '12h',
    username_separator: stored?.username_separator ?? (stored?.username_colon ? 'colon' : 'none'),
    username_style: stored?.username_style ?? 'plain',
    username_accent_source: stored?.username_accent_source ?? 'user',
    mod_action_style: stored?.mod_action_style ?? (stored?.drag_moderation_enabled === false ? 'buttons' : 'both'),
    mod_drag_layout: stored?.mod_drag_layout ?? 'column',
    mod_pin_style: stored?.mod_pin_style ?? 'both',
    emote_scale: stored?.emote_scale ?? 1,
    animate_emotes: stored?.animate_emotes ?? 'always',
    show_chat_gifs: stored?.show_chat_gifs ?? true,
    backfill_opacity: stored?.backfill_opacity ?? 100,
    emote_margin: stored?.emote_margin ?? 0.125,
    emote_hover_size: stored?.emote_hover_size ?? 96,
    deleted_message_style: stored?.deleted_message_style ?? 'strikethrough',
    hide_shared_chat: stored?.hide_shared_chat ?? false,
    paint_mentions_in_body: stored?.paint_mentions_in_body ?? true,
    compact_emote_tooltips: stored?.compact_emote_tooltips ?? false,
    ffz_emote_effects: stored?.ffz_emote_effects ?? true,
    bttv_emote_modifiers: stored?.bttv_emote_modifiers ?? true,
    giant_emotes: stored?.giant_emotes ?? true,
    user_card_opens_messages: stored?.user_card_opens_messages ?? false,
    seventv_emote_notices: stored?.seventv_emote_notices ?? true,
    link_previews: stored?.link_previews ?? true,
    link_preview_keep_link: stored?.link_preview_keep_link ?? false,
    shorten_links: stored?.shorten_links ?? true,
    link_preview_trusted_domains: stored?.link_preview_trusted_domains ?? [],
    pinned_collapsed_style: stored?.pinned_collapsed_style ?? 'bar',
    pinned_start_collapsed: stored?.pinned_start_collapsed ?? true,
    polls_start_collapsed: stored?.polls_start_collapsed ?? false,
    name_color_adjustment: (stored?.name_color_adjustment ?? 'hsl_loop') as 'off' | 'hsl_loop',
    show_badges: stored?.show_badges ?? true,
    badge_scale: stored?.badge_scale ?? 1,
    show_third_party_badges: stored?.show_third_party_badges ?? true,
    hidden_badge_providers: stored?.hidden_badge_providers ?? [],
    message_entrance: (stored?.message_entrance ?? 'none') as 'none' | 'fade' | 'slide' | 'rise',
    emoji_style: (stored?.emoji_style ?? 'apple') as 'system' | 'apple' | 'google' | 'twitter' | 'facebook',
    show_personal_emotes: stored?.show_personal_emotes ?? true,
    giant_emote_align: (stored?.giant_emote_align ?? 'center') as 'left' | 'center' | 'right' | 'inline',
    show_avatars: stored?.show_avatars ?? true,
    show_at_sign: stored?.show_at_sign ?? false,
    reply_style: (stored?.reply_style ?? 'full') as 'full' | 'mention' | 'off',
    link_color: stored?.link_color ?? '',
    link_underline: stored?.link_underline ?? true,
    // Absent stays absent: every save writes `cd` whole, so a field missing
    // here would be wiped, and a default written here would change what an
    // untouched install sees.
    mention_sound: stored?.mention_sound,
    mention_sound_volume: stored?.mention_sound_volume,
    mention_sound_replies: stored?.mention_sound_replies,
    mention_weight: stored?.mention_weight,
    mention_italic: stored?.mention_italic,
    mention_style: stored?.mention_style,
    mention_text_color: stored?.mention_text_color,
    chat_dock_switcher: stored?.chat_dock_switcher,
    username_colon: stored?.username_colon,
    drag_moderation_enabled: stored?.drag_moderation_enabled,
  };
  const badgeProviderHidden = (id: string) => cd.hidden_badge_providers.includes(id);
  const toggleBadgeProvider = (id: string) =>
    setDesign({
      hidden_badge_providers: badgeProviderHidden(id)
        ? cd.hidden_badge_providers.filter((k) => k !== id)
        : [...cd.hidden_badge_providers, id],
    });

  const setDesign = (patch: Partial<typeof cd>) => {
    updateSettings({
      ...settings,
      chat_design: { ...cd, ...patch },
    });
  };

  const rp = settings.message_repeat;
  const repeatMode: RepeatDisplayMode = rp?.mode ?? 'off';
  const repeatThreshold = Math.max(2, rp?.threshold ?? 2);
  const repeatWindow = rp?.window_seconds ?? 60;
  const setRepeat = (patch: Partial<MessageRepeatSettings>) =>
    updateSettings({
      ...settings,
      message_repeat: { ...settings.message_repeat, ...patch },
    });

  const cfs = settings.chat_filters;
  const setChatFilters = (patch: Partial<ChatFilterSettings>) =>
    updateSettings({
      ...settings,
      chat_filters: { ...settings.chat_filters, ...patch },
    });
  const setHidden = (name: string, scope: { provider: ProviderId; channel: string } | 'global', hidden: boolean) =>
    invoke('set_chat_user_hidden', {
      name,
      channelKey: scope === 'global' ? null : filterChannelKey(scope.provider, scope.channel),
      hidden,
    }).catch((err) => Logger.warn('[ChatSettings] set_chat_user_hidden failed:', err));
  // Per-channel entries flattened for display: [channelKey, label, names].
  const perChannelHidden = Object.entries(cfs?.per_channel ?? {})
    .map(([key, names]) => {
      const pk = parseKey(key);
      const label = pk.provider === 'twitch' ? pk.channel : `${pk.channel} (${pk.provider})`;
      return { key, pk, label, names: names ?? [] };
    })
    .filter((e) => e.names.length > 0)
    .sort((a, b) => a.label.localeCompare(b.label));

  const setUserCard = (patch: Partial<UserCardSettings>) =>
    updateSettings({
      ...settings,
      user_card: { ...settings.user_card, ...patch },
    });

  const setInput = (patch: Partial<NonNullable<typeof settings.chat_input>>) =>
    updateSettings({
      ...settings,
      chat_input: { ...settings.chat_input, ...patch },
    });

  const setBlend = (patch: Partial<NonNullable<typeof settings.chat_blend>>) =>
    updateSettings({
      ...settings,
      chat_blend: { ...settings.chat_blend, ...patch },
    });

  const setRender = (patch: Partial<NonNullable<typeof settings.chat_render>>) =>
    updateSettings({
      ...settings,
      chat_render: { ...settings.chat_render, ...patch },
    });

  const setCosmetics = (patch: Partial<NonNullable<typeof settings.cosmetics>>) =>
    updateSettings({
      ...settings,
      cosmetics: { ...settings.cosmetics, ...patch },
    });

  const logging = settings.chat_logging ?? {};
  const loggingEnabled = logging.enabled ?? false;
  const setLogging = (patch: Partial<NonNullable<typeof settings.chat_logging>>) =>
    updateSettings({
      ...settings,
      chat_logging: { ...logging, ...patch },
    });

  // The folder logs land in right now (custom or default), resolved by the
  // backend so the displayed path always matches what the writer uses.
  const [logDir, setLogDir] = useState('');
  useEffect(() => {
    invoke<string>('get_chat_log_dir')
      .then(setLogDir)
      .catch(() => setLogDir(''));
  }, [logging.folder, loggingEnabled]);

  const browseLogFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({ directory: true, multiple: false });
      if (typeof picked === 'string' && picked) setLogging({ folder: picked });
    } catch {
      // Dialog dismissed or unavailable; keep the current folder.
    }
  };

  const openLogFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      if (logDir) await open(logDir);
    } catch {
      // The folder appears once the first line is logged.
    }
  };

  return (
    <div className="space-y-8">
      {/* Desktop only. This positions the MAIN app's chat panel (left / right /
          bottom / hidden) plus the hover-reveal that goes with it. The phone
          shell stacks the player over chat and has no edge to tuck against. */}
      {!hidePlacement && !IS_MOBILE && (
      <>
      <SettingsSection
        label="Chat Placement"
        description="Where chat sits next to the player."
      >
        <SettingsRow
          title="Position"
          onReset={resetFor('chat_placement')}
          description="Dock chat to the left, right, or bottom of the player, or hide it to give the video the whole window."
          help="Hover reveal (left or right only) keeps chat tucked against its edge and slides it out when you move toward that side. The player shrinks to make room, the same as dragging the chat open."
        >
          <SegmentedSelect<'left' | 'right' | 'bottom' | 'hidden'>
            value={settings.chat_placement as 'left' | 'right' | 'bottom' | 'hidden'}
            onChange={(placement) => updateSettings({ ...settings, chat_placement: placement })}
            options={[
              { value: 'hidden', label: 'Hidden' },
              { value: 'bottom', label: 'Bottom' },
              { value: 'left', label: 'Left' },
              { value: 'right', label: 'Right' },
            ]}
          />
          {(settings.chat_placement === 'left' || settings.chat_placement === 'right') && (
            <SubControls>
              <SubControl
                title="Hover reveal"
                onReset={resetFor(['chat_auto_hide', false])}
                control={
                  <Toggle
                    enabled={settings.chat_auto_hide ?? false}
                    onChange={() =>
                      updateSettings({ ...settings, chat_auto_hide: !(settings.chat_auto_hide ?? false) })
                    }
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Fullscreen Chat"
        description="Keep chatting while the stream fills the screen."
      >
        <SettingsRow
          title="Overlay"
          onReset={resetFor(['fullscreen_chat.mode', 'overlay'])}
          description="The chat panel floats over fullscreen video as a translucent column. No extra window."
          help="Auto-hide fades the column out with the player controls and brings it back when you move the mouse; hovering the chat or typing keeps it up. Opacity is how solid the column is. Width runs from 240 to 640 pixels. Side on Auto follows the chat placement, so a bottom-docked chat floats on the right."
          control={
            <Toggle
              enabled={(settings.fullscreen_chat?.mode ?? 'overlay') === 'overlay'}
              onChange={() =>
                updateSettings({
                  ...settings,
                  fullscreen_chat: {
                    ...settings.fullscreen_chat,
                    mode: (settings.fullscreen_chat?.mode ?? 'overlay') === 'overlay' ? 'hidden' : 'overlay',
                  },
                })
              }
            />
          }
        >
          {(settings.fullscreen_chat?.mode ?? 'overlay') === 'overlay' && (
            <SubControls>
              <SubControl
                title="Auto-hide"
                onReset={resetFor(['fullscreen_chat.auto_hide', true])}
                control={
                  <Toggle
                    enabled={settings.fullscreen_chat?.auto_hide ?? true}
                    onChange={() =>
                      updateSettings({
                        ...settings,
                        fullscreen_chat: {
                          ...settings.fullscreen_chat,
                          auto_hide: !(settings.fullscreen_chat?.auto_hide ?? true),
                        },
                      })
                    }
                  />
                }
              />
              <SubControl
                title="Opacity"
                onReset={resetFor(['fullscreen_chat.opacity', 55])}
                control={
                  <InlineSlider
                    value={settings.fullscreen_chat?.opacity ?? 55}
                    min={0}
                    max={100}
                    step={5}
                    label="Fullscreen chat opacity"
                    format={(v) => `${v}%`}
                    onChange={(opacity) =>
                      updateSettings({ ...settings, fullscreen_chat: { ...settings.fullscreen_chat, opacity } })
                    }
                  />
                }
              />
              <SubControl
                title="Width"
                onReset={resetFor(['fullscreen_chat.width', 340])}
                control={
                  <input
                    type="number"
                    min={240}
                    max={640}
                    step={10}
                    aria-label="Fullscreen chat width in pixels"
                    value={settings.fullscreen_chat?.width ?? 340}
                    onChange={(e) => {
                      const n = Math.max(240, Math.min(640, Math.round(Number(e.target.value) || 340)));
                      updateSettings({
                        ...settings,
                        fullscreen_chat: { ...settings.fullscreen_chat, width: n },
                      });
                    }}
                    className="glass-input w-24 px-2.5 py-1.5 text-sm text-textPrimary"
                  />
                }
              />
              <SubControl
                title="Side"
                onReset={resetFor(['fullscreen_chat.side', 'auto'])}
                control={
                  <SegmentedSelect<'auto' | 'left' | 'right'>
                    value={settings.fullscreen_chat?.side ?? 'auto'}
                    onChange={(side) =>
                      updateSettings({ ...settings, fullscreen_chat: { ...settings.fullscreen_chat, side } })
                    }
                    options={[
                      { value: 'auto', label: 'Auto' },
                      { value: 'left', label: 'Left' },
                      { value: 'right', label: 'Right' },
                    ]}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>
      </>
      )}

      {/* The phone's counterpart to Chat Placement: how chat shares the screen
          with landscape video. The overlay's opacity, width and side are the
          SAME keys as the desktop overlay, so one preference follows you
          between the two. */}
      {IS_MOBILE && (
        <SettingsSection
          label="Landscape Chat"
          description="How chat shares the screen when the phone is on its side."
        >
          <SettingsRow
            title="Layout"
            onReset={resetFor(['fullscreen_chat.phone_layout', 'overlay'])}
            description="Turn the phone sideways and tap the chat button on the player. Chat floats over the video, or takes a column beside it."
            help="Drag the column's edge to resize it either way. Floating chat is read-only; tap its edge for the background slider. A lower background lets more of the video show through. Width never goes past half the screen."
          >
            <SegmentedSelect<'overlay' | 'beside'>
              value={settings.fullscreen_chat?.phone_layout ?? 'overlay'}
              onChange={(phone_layout) =>
                updateSettings({
                  ...settings,
                  fullscreen_chat: { ...settings.fullscreen_chat, phone_layout },
                })
              }
              options={[
                { value: 'overlay', label: 'Over the video' },
                { value: 'beside', label: 'Beside the video' },
              ]}
            />
            <SubControls>
              {(settings.fullscreen_chat?.phone_layout ?? 'overlay') === 'overlay' && (
                <SubControl
                  title="Background"
                  onReset={resetFor(['fullscreen_chat.opacity', 55])}
                  control={
                    <InlineSlider
                      value={settings.fullscreen_chat?.opacity ?? 55}
                      min={0}
                      max={100}
                      step={5}
                      label="Landscape chat background"
                      format={(v) => `${v}%`}
                      onChange={(opacity) =>
                        updateSettings({ ...settings, fullscreen_chat: { ...settings.fullscreen_chat, opacity } })
                      }
                    />
                  }
                />
              )}
              <SubControl
                title="Width"
                onReset={resetFor(['fullscreen_chat.width', 340])}
                control={
                  <InlineSlider
                    value={Math.min(480, settings.fullscreen_chat?.width ?? 340)}
                    min={240}
                    max={480}
                    step={20}
                    label="Landscape chat width"
                    format={(v) => `${v}px`}
                    onChange={(width) =>
                      updateSettings({ ...settings, fullscreen_chat: { ...settings.fullscreen_chat, width } })
                    }
                  />
                }
              />
              <SubControl
                title="Side"
                onReset={
                  settings.fullscreen_chat?.side === 'left' ? resetFor(['fullscreen_chat.side', 'auto']) : undefined
                }
                control={
                  <SegmentedSelect<'left' | 'right'>
                    value={settings.fullscreen_chat?.side === 'left' ? 'left' : 'right'}
                    onChange={(side) =>
                      updateSettings({ ...settings, fullscreen_chat: { ...settings.fullscreen_chat, side } })
                    }
                    options={[
                      { value: 'left', label: 'Left' },
                      { value: 'right', label: 'Right' },
                    ]}
                  />
                }
              />
            </SubControls>
          </SettingsRow>
        </SettingsSection>
      )}

      <SettingsSection
        id="settings-section-combined-chat"
        label="Combined Chat"
        description="Show a streamer's chat from their other platforms alongside the one you are watching."
      >
        <SettingsRow
          title="Merged feed"
          onReset={resetFor(['chat_blend.enabled', false])}
          description="When a streamer you are watching also streams elsewhere, their other chats join this one in a single feed, each message marked with where it came from."
          help="Off by default, and inactive while off: no extra connections and nothing extra fetched. Suggestions look for a Kick or YouTube channel of the same name when you open a stream and wait behind the + in the chat header; YouTube is only found while it is live, and nothing is linked without you. Platform marks put a small logo on messages from the other platforms. Platforms sets which ones may ever join; the marks in the chat header drop one for just this channel. You chat on the platform you are watching, but a reply to someone elsewhere goes back there. Saved filters about sub length or bits only match Twitch messages."
          control={
            <Toggle
              enabled={settings.chat_blend?.enabled === true}
              onChange={() => setBlend({ enabled: !(settings.chat_blend?.enabled === true) })}
            />
          }
        >
          {settings.chat_blend?.enabled === true && (
            <SubControls>
              <SubControl
                title="Suggestions"
                onReset={resetFor(['chat_blend.suggest_links', true])}
                control={
                  <Toggle
                    enabled={settings.chat_blend?.suggest_links !== false}
                    onChange={() => setBlend({ suggest_links: !(settings.chat_blend?.suggest_links !== false) })}
                  />
                }
              />
              <SubControl
                title="Platform marks"
                onReset={resetFor(['chat_blend.show_platform_badge', true])}
                control={
                  <Toggle
                    enabled={settings.chat_blend?.show_platform_badge !== false}
                    onChange={() =>
                      setBlend({ show_platform_badge: !(settings.chat_blend?.show_platform_badge !== false) })
                    }
                  />
                }
              />
              <SubControl
                title="Platforms"
                onReset={
                  BLEND_PLATFORMS.some((p) => settings.chat_blend?.platforms?.[p] === false)
                    ? resetFor(['chat_blend.platforms', {}])
                    : undefined
                }
                stacked
                control={
                  <div className="flex flex-wrap gap-4">
                    {BLEND_PLATFORMS.map((p) => (
                      <label key={p} className="flex items-center gap-1.5 text-[12px] text-textSecondary">
                        <Toggle
                          enabled={settings.chat_blend?.platforms?.[p] !== false}
                          ariaLabel={`Include ${PROVIDERS[p].label} in combined chat`}
                          onChange={() =>
                            setBlend({
                              platforms: {
                                ...settings.chat_blend?.platforms,
                                [p]: !(settings.chat_blend?.platforms?.[p] !== false),
                              },
                            })
                          }
                        />
                        {PROVIDERS[p].label}
                      </label>
                    ))}
                  </div>
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-youtube-chat"
        label="YouTube Chat"
        description="Settings that only apply to YouTube chat: which of its two feeds you read, and how Super Chat amounts show."
      >
        <SettingsRow
          title="Feed"
          onReset={resetFor(['youtube_chat_view', 'live'])}
          description="Live chat shows everything, while Top chat is YouTube's own filtered view that keeps a very fast chat readable."
          help="Top chat drops messages YouTube judges low quality and most of one person's repeats, so you see less but can miss some."
        >
          <SegmentedSelect<YouTubeChatView>
            value={settings.youtube_chat_view ?? 'live'}
            onChange={(view) => updateSettings({ ...settings, youtube_chat_view: view })}
            options={[
              { value: 'live', label: 'Live chat' },
              { value: 'top', label: 'Top chat' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Currency"
          onReset={resetFor(['chat_events.superchat_currency', ''])}
          description="Show Super Chat amounts converted to one currency. Rates refresh daily; until they load the amount shows as sent."
        >
          <Dropdown<string>
            value={chatEvents.superchat_currency ?? ''}
            onChange={(superchat_currency) => setEvents({ superchat_currency })}
            className="w-full"
            ariaLabel="Super Chat currency"
            options={[{ value: '', label: 'As sent' }, ...CURRENCY_OPTIONS.map((c) => ({ value: c, label: c }))]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Polls & Predictions"
        description="The live cards at the top of chat while the streamer runs a poll or a prediction."
      >
        <SettingsRow
          title="Polls"
          onReset={resetFor(['show_polls', true])}
          description="A live poll card with the running vote tally."
          help="Collapsed opens a poll as its header bar, so it never takes over the top of chat; tap the header to expand it. Collapsing a poll sticks: it no longer reopens every time somebody votes."
          control={
            <Toggle
              enabled={settings.show_polls ?? true}
              onChange={() => updateSettings({ ...settings, show_polls: !(settings.show_polls ?? true) })}
            />
          }
        >
          {(settings.show_polls ?? true) && (
            <SubControls>
              <SubControl
                title="Collapsed"
                onReset={resetFor(['chat_design.polls_start_collapsed', false])}
                control={
                  <Toggle
                    enabled={cd.polls_start_collapsed ?? false}
                    onChange={() =>
                      setDesign({ polls_start_collapsed: !(cd.polls_start_collapsed ?? false) })
                    }
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="Predictions"
          onReset={resetFor(['show_predictions', true])}
          description="A live prediction card with the outcomes and how points are stacking up."
          help="Top card picks which one sits on top when a poll and a prediction run at the same time. Both cards show either way, stacked."
          control={
            <Toggle
              enabled={settings.show_predictions ?? true}
              onChange={() =>
                updateSettings({ ...settings, show_predictions: !(settings.show_predictions ?? true) })
              }
            />
          }
        >
          {(settings.show_polls ?? true) && (settings.show_predictions ?? true) && (
            <SubControls>
              <SubControl
                title="Top card"
                onReset={resetFor(['chat_overlay_order', 'prediction-first'])}
                control={
                  <SegmentedSelect<'prediction-first' | 'poll-first'>
                    value={settings.chat_overlay_order ?? 'prediction-first'}
                    onChange={(order) => updateSettings({ ...settings, chat_overlay_order: order })}
                    options={[
                      { value: 'prediction-first', label: 'Prediction' },
                      { value: 'poll-first', label: 'Poll' },
                    ]}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-chat-events"
        label="Chat Events"
        description="What live channel activity shows while you watch. Turn any of these off to keep chat clean."
      >
        <SettingsRow
          title="Redemptions"
          onReset={resetFor(['show_channel_point_redemptions', true])}
          description="A chat row when someone redeems a channel point reward that does not post its own message, like a no-input reward."
          help="Rewards that already post to chat are unaffected."
          control={
            <Toggle
              enabled={settings.show_channel_point_redemptions ?? true}
              onChange={() =>
                updateSettings({
                  ...settings,
                  show_channel_point_redemptions: !(settings.show_channel_point_redemptions ?? true),
                })
              }
            />
          }
        />

        <SettingsRow
          title="Gift sub batches"
          onReset={resetFor(['collapse_gift_subs', true])}
          description="One 'gifting N subs' row with the recipients attached when someone gifts a batch, instead of a row per gift."
          help="Turn this off to see every gift as its own row."
          control={
            <Toggle
              enabled={settings.collapse_gift_subs ?? true}
              onChange={() =>
                updateSettings({ ...settings, collapse_gift_subs: !(settings.collapse_gift_subs ?? true) })
              }
            />
          }
        />

        <SettingsRow
          title="Clip chat replay"
          onReset={resetFor(['clip_chat_replay', true])}
          description="Shows the chat that was live while a clip was recorded, beside the clip."
          help="Needs the original broadcast to still be up, so older clips may have no replay."
          control={
            <Toggle
              enabled={settings.clip_chat_replay ?? true}
              onChange={() =>
                updateSettings({ ...settings, clip_chat_replay: !(settings.clip_chat_replay ?? true) })
              }
            />
          }
        />

        <SettingsRow
          title="Style"
          onReset={resetFor(['chat_events.event_style', 'cards'])}
          description="Subs, gifts, bits and milestones as tinted cards, as a plain row with a ring, or as a plain row."
          help="With Outline, Outline color sets the ring. Leave it on the default to follow the theme accent."
        >
          <div className="space-y-3">
            <SegmentedSelect<'cards' | 'outline' | 'plain'>
              value={chatEvents.event_style ?? 'cards'}
              onChange={(event_style) => setEvents({ event_style })}
              onPreview={setEventStyleHover}
              options={[
                { value: 'cards', label: 'Cards' },
                { value: 'outline', label: 'Outline' },
                { value: 'plain', label: 'Plain' },
              ]}
            />
            <EventRowPreview
              design={cd}
              events={{
                ...chatEvents,
                event_style: eventStyleHover ?? chatEvents.event_style,
                event_animation: glintHover ?? chatEvents.event_animation,
              }}
            />
          </div>
          {(chatEvents.event_style ?? 'cards') === 'outline' && (
            <SubControls>
              <SubControl
                title="Outline color"
                onReset={resetFor(['chat_events.event_outline_color', ''])}
                control={
                  <ColorSwatch
                    value={chatEvents.event_outline_color || '#9147ff'}
                    onChange={(color) => setEvents({ event_outline_color: color })}
                    tooltip="Outline color"
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="Glint"
          onReset={resetFor(['chat_events.event_animation', 'none'])}
          description="A short highlight when an event row lands: a sheen across it, a pulse, or a spark that runs around the edge."
          help="Loop keeps the glint going; off plays it once as the row arrives."
        >
          <Dropdown<'none' | 'sheen' | 'pulse' | 'chase'>
            value={chatEvents.event_animation ?? 'none'}
            onChange={(event_animation) => setEvents({ event_animation })}
            onPreview={setGlintHover}
            className="w-full"
            ariaLabel="Event glint"
            options={[
              { value: 'none', label: 'None' },
              { value: 'sheen', label: 'Sheen' },
              { value: 'pulse', label: 'Pulse' },
              { value: 'chase', label: 'Chase' },
            ]}
          />
          {(chatEvents.event_animation ?? 'none') !== 'none' && (
            <SubControls>
              <SubControl
                title="Loop"
                onReset={resetFor(['chat_events.event_animate_repeat', false])}
                control={
                  <Toggle
                    enabled={chatEvents.event_animate_repeat ?? false}
                    onChange={() => setEvents({ event_animate_repeat: !(chatEvents.event_animate_repeat ?? false) })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="Cheers"
          onReset={resetFor(['chat_events.cheer_display', 'card'])}
          description="Bits cheers as their own card with the cheer gem, or as an ordinary message with the cheermotes inline."
        >
          <SegmentedSelect<'card' | 'message'>
            value={chatEvents.cheer_display ?? 'card'}
            onChange={(cheer_display) => setEvents({ cheer_display })}
            options={[
              { value: 'card', label: 'Card' },
              { value: 'message', label: 'Message' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Wording"
          description="Your own sentence for each kind of event. Tokens in braces fill in from the event; if one is missing, the platform's wording is used."
          help="Tokens: {username} {tier} {months} {years} {streak} {recipient} {count} {bits} {viewers} {channel} {platform} {time} {default}. Leave a box empty to keep the platform's wording."
        >
          <div className="flex flex-col gap-2 w-full">
            {(['subscription', 'gift', 'cheer', 'milestone'] as ChatEventCategory[]).map((cat) => (
              <label key={cat} className="flex flex-col gap-1">
                <span className="text-[11px] font-semibold uppercase tracking-wide text-textMuted">
                  {EVENT_CATEGORIES.find((c) => c.id === cat)?.label ?? cat}
                </span>
                <input
                  type="text"
                  value={chatEvents.event_templates?.[cat] ?? ''}
                  placeholder={EVENT_TEMPLATE_EXAMPLES[cat]}
                  maxLength={200}
                  onChange={(e) =>
                    setEvents({ event_templates: { ...chatEvents.event_templates, [cat]: e.target.value } })
                  }
                  className="glass-input w-full px-3 py-2 text-[13px] text-textPrimary placeholder:text-textMuted"
                />
              </label>
            ))}
          </div>
        </SettingsRow>

        <SettingsRow
          title="Platforms"
          onReset={resetFor(['chat_events.hidden_provider_events', []])}
          description="Turn event kinds off per platform. Lit means shown."
        >
          <div className="flex flex-col gap-2 w-full">
            {(Object.keys(PROVIDER_EVENT_CATEGORIES) as Array<keyof typeof PROVIDER_EVENT_CATEGORIES>).map((provider) => (
              <div key={provider} className="flex flex-wrap items-center gap-1.5">
                <span className="text-[12px] text-textSecondary w-16 shrink-0">{PROVIDER_LABELS[provider] ?? provider}</span>
                {(PROVIDER_EVENT_CATEGORIES[provider] ?? []).map((cat) => {
                  const key = `${provider}:${cat}`;
                  const on = !hiddenEvents.includes(key);
                  const label = PROVIDER_CATEGORY_LABELS[provider]?.[cat] ?? EVENT_CATEGORIES.find((c) => c.id === cat)?.label ?? cat;
                  return (
                    <button
                      key={key}
                      type="button"
                      aria-pressed={on}
                      onClick={() => toggleHiddenEvent(key)}
                      className={`px-2.5 py-1 rounded-full text-[12px] font-medium transition-colors ${
                        on ? 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textPrimary' : 'glass-button-static text-textMuted'
                      }`}
                    >
                      {label}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Pinned Messages"
        description="How a pinned message shows at the top of chat."
      >
        <SettingsRow
          title="Collapsed"
          onReset={resetFor(['chat_design.pinned_start_collapsed', true])}
          description="Shows the pinned message as a compact one-line bar when you enter a channel."
          help="Click the bar to expand it. Turn this off to always open pins fully expanded. Style shrinks any collapsed pin to a thin bar showing the sender and the start of the message, or hides it completely."
          control={
            <Toggle
              enabled={cd.pinned_start_collapsed ?? true}
              onChange={() => setDesign({ pinned_start_collapsed: !(cd.pinned_start_collapsed ?? true) })}
            />
          }
        >
          <SubControls>
            <SubControl
              title="Style"
              onReset={resetFor(['chat_design.pinned_collapsed_style', 'bar'])}
              control={
                <SegmentedSelect<'bar' | 'hidden'>
                  value={cd.pinned_collapsed_style ?? 'bar'}
                  onChange={(v) => setDesign({ pinned_collapsed_style: v })}
                  options={[
                    { value: 'bar', label: 'Bar' },
                    { value: 'hidden', label: 'Hidden' },
                  ]}
                />
              }
            />
          </SubControls>
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Message Layout"
        description="Spacing, text size, animation and timestamps."
      >
        <div className="pt-3">
          <MessageLayoutPreview design={cd} entrance={entranceHover ?? cd.message_entrance} />
        </div>

        <SettingsRow
          title="Dividers"
          onReset={resetFor(['chat_design.show_dividers', true])}
          description="A thin line between messages so a fast chat is easier to scan."
          control={
            <Toggle
              enabled={cd.show_dividers ?? true}
              onChange={() => setDesign({ show_dividers: !(cd.show_dividers ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Striped rows"
          onReset={resetFor(['chat_design.alternating_backgrounds', false])}
          description="Gives every other message a slightly different background, in your theme's colors, so rows are easier to follow."
          control={
            <Toggle
              enabled={cd.alternating_backgrounds ?? false}
              onChange={() => setDesign({ alternating_backgrounds: !(cd.alternating_backgrounds ?? false) })}
            />
          }
        />

        <SettingsRow
          title={`Spacing: ${cd.message_spacing ?? 8}px`}
          onReset={resetFor(['chat_design.message_spacing', 8])}
          description="Blank space between messages; more room means fewer messages on screen."
        >
          <input
            type="range"
            min="0"
            max="20"
            step="1"
            value={cd.message_spacing ?? 8}
            onChange={(e) => setDesign({ message_spacing: parseInt(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title={`Text size: ${cd.font_size ?? 14}px`}
          onReset={resetFor(['chat_design.font_size', 14])}
          description="Size of message text, with room to go large when MultiChat fills a whole monitor."
        >
          <input
            type="range"
            min="10"
            max="48"
            step="1"
            value={cd.font_size ?? 14}
            onChange={(e) => setDesign({ font_size: parseInt(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        {/* Desktop only: its own description says MultiChat, and MultiChat is
            gated off mobile entirely. Note this is NOT the phone's Activity tab,
            which is drops and badges and takes no sizing from here. */}
        {!IS_MOBILE && (
          <SettingsRow
            title={`Activity feed size: ${cd.activity_font_size ?? 14}px`}
            onReset={resetFor(['chat_design.activity_font_size', 14])}
            description="Text size for the MultiChat activity feed, where subs, raids, and gifts land."
          >
            <input
              type="range"
              min="10"
              max="28"
              step="1"
              value={cd.activity_font_size ?? 14}
              onChange={(e) => setDesign({ activity_font_size: parseInt(e.target.value) })}
              className="w-full accent-accent cursor-pointer"
            />
          </SettingsRow>
        )}

        <SettingsRow
          title="Text weight"
          onReset={resetFor(['chat_design.font_weight', 400])}
          description="How heavy the message text is, from light to bold."
        >
          <Dropdown
            value={cd.font_weight ?? 400}
            onChange={(v) => setDesign({ font_weight: v })}
            className="w-full"
            ariaLabel="Font weight"
            options={[
              { value: 300, label: 'Light (300)' },
              { value: 400, label: 'Normal (400)' },
              { value: 500, label: 'Medium (500)' },
              { value: 600, label: 'Semi-Bold (600)' },
              { value: 700, label: 'Bold (700)' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Message animation"
          onReset={resetFor(['chat_design.message_entrance', 'none'])}
          description="How a new message arrives: a short fade or slide as each one lands. History loaded on join never animates, and it is skipped when motion is reduced."
        >
          <SegmentedSelect<'none' | 'fade' | 'slide' | 'rise'>
            value={cd.message_entrance}
            onChange={(message_entrance) => setDesign({ message_entrance })}
            onPreview={setEntranceHover}
            options={[
              { value: 'none', label: 'Instant' },
              { value: 'fade', label: 'Fade' },
              { value: 'slide', label: 'Slide' },
              { value: 'rise', label: 'Rise' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title={`History opacity: ${cd.backfill_opacity ?? 100}%`}
          onReset={resetFor(['chat_design.backfill_opacity', 100])}
          description="Dim the scrollback that loads when you join a chat, so live messages stand out."
        >
          <input
            type="range"
            min={30}
            max={100}
            step={5}
            value={cd.backfill_opacity ?? 100}
            onChange={(e) => setDesign({ backfill_opacity: Number(e.target.value) })}
            className="w-40 accent-accent"
          />
        </SettingsRow>

        <SettingsRow
          title="Timestamps"
          onReset={resetFor(['chat_design.show_timestamps', false])}
          description="Shows the time each message was sent, next to the name."
          help="Clock is 12-hour (7:42 PM) or 24-hour (19:42). Seconds turns 7:42 PM into 7:42:30 PM."
          control={
            <Toggle
              enabled={cd.show_timestamps ?? false}
              onChange={() => setDesign({ show_timestamps: !(cd.show_timestamps ?? false) })}
            />
          }
        >
          {cd.show_timestamps && (
            <SubControls>
              <SubControl
                title="Clock"
                onReset={resetFor(['chat_design.timestamp_format', '12h'])}
                control={
                  <SegmentedSelect<'12h' | '24h'>
                    value={cd.timestamp_format ?? '12h'}
                    onChange={(timestamp_format) => setDesign({ timestamp_format })}
                    options={[
                      { value: '12h', label: '12h' },
                      { value: '24h', label: '24h' },
                    ]}
                  />
                }
              />
              <SubControl
                title="Seconds"
                onReset={resetFor(['chat_design.show_timestamp_seconds', false])}
                control={
                  <Toggle
                    enabled={cd.show_timestamp_seconds ?? false}
                    onChange={() => setDesign({ show_timestamp_seconds: !(cd.show_timestamp_seconds ?? false) })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Names & Badges"
        description="How chatter names, badges and 7TV paints look."
      >
        <SettingPreview caption="how your name looks in chat">
          <NamePrefixPreview
            separator={cd.username_separator ?? 'none'}
            nameStyle={cd.username_style ?? 'plain'}
            accentSource={cd.username_accent_source ?? 'user'}
          />
        </SettingPreview>

        <SettingsRow
          title="Name style"
          onReset={resetFor(['chat_design.username_style', 'plain'])}
          description="How names stand out from the message: plain, or with a bar, chip, brackets, or dot."
          help="Separator is the mark between a name and its message, like a colon or an arrow; /me messages never get one. Prefix color paints the separator, bar, dot, brackets, or chip with the chatter's own color or your theme accent."
        >
          <Dropdown<'plain' | 'bar' | 'chip' | 'brackets' | 'dot'>
            value={cd.username_style ?? 'plain'}
            onChange={(v) => setDesign({ username_style: v })}
            className="w-full"
            ariaLabel="Name style"
            options={[
              { value: 'plain', label: 'Plain' },
              { value: 'bar', label: 'Accent bar' },
              { value: 'chip', label: 'Chip / tag' },
              { value: 'brackets', label: 'Brackets   [name]' },
              { value: 'dot', label: 'Color dot' },
            ]}
          />
          <SubControls>
            <SubControl
              title="Separator"
              onReset={resetFor(['chat_design.username_separator', 'none'])}
              control={
                <Dropdown<'none' | 'colon' | 'dot' | 'arrow' | 'pipe' | 'dash'>
                  value={cd.username_separator ?? 'none'}
                  onChange={(v) => setDesign({ username_separator: v })}
                  className="w-44"
                  ariaLabel="Name separator"
                  options={[
                    { value: 'none', label: 'None' },
                    { value: 'colon', label: 'Colon   name:' },
                    { value: 'dot', label: 'Dot   name ·' },
                    { value: 'arrow', label: 'Arrow   name ›' },
                    { value: 'pipe', label: 'Pipe   name |' },
                    { value: 'dash', label: 'Dash   name –' },
                  ]}
                />
              }
            />
            {(cd.username_separator !== 'none' || cd.username_style !== 'plain') && (
              <SubControl
                title="Prefix color"
                onReset={resetFor(['chat_design.username_accent_source', 'user'])}
                control={
                  <SegmentedSelect<'user' | 'theme'>
                    value={cd.username_accent_source ?? 'user'}
                    onChange={(v) => setDesign({ username_accent_source: v })}
                    options={[
                      { value: 'user', label: 'User color' },
                      { value: 'theme', label: 'Theme accent' },
                    ]}
                  />
                }
              />
            )}
          </SubControls>
        </SettingsRow>

        <SettingsRow
          title="Readable colors"
          onReset={resetFor(['chat_design.name_color_adjustment', 'hsl_loop'])}
          description="Nudges a chatter's color lighter on a dark theme, or darker on a light one, until it stands out from the background. The hue stays theirs. Off shows colors exactly as they set them."
          control={
            <Toggle
              enabled={(cd.name_color_adjustment ?? 'hsl_loop') !== 'off'}
              onChange={() =>
                setDesign({
                  name_color_adjustment: (cd.name_color_adjustment ?? 'hsl_loop') === 'off' ? 'hsl_loop' : 'off',
                })
              }
            />
          }
        />

        <SettingsRow
          title="Badges"
          onReset={resetFor(['chat_design.show_badges', true])}
          description="The platform's own badges next to names: moderator, subscriber, VIP and the rest."
          control={<Toggle enabled={cd.show_badges} onChange={() => setDesign({ show_badges: !cd.show_badges })} />}
        />

        <SettingsRow
          title="Badge size"
          onReset={resetFor(['chat_design.badge_scale', 1])}
          description="How big badges draw, relative to the text."
          control={
            <div className="flex items-center gap-2">
              <input
                type="range"
                min={0.5}
                max={2.5}
                step={0.05}
                value={cd.badge_scale}
                disabled={!cd.show_badges && !cd.show_third_party_badges}
                onChange={(e) => setDesign({ badge_scale: Number(e.target.value) })}
                className="w-32 accent-accent disabled:opacity-40"
              />
              <span className="text-[12px] text-textMuted tabular-nums w-10 text-right">{cd.badge_scale.toFixed(2)}x</span>
            </div>
          }
        />

        <SettingsRow
          title="Add-on badges"
          onReset={resetFor(['chat_design.show_third_party_badges', true])}
          description="Badges from 7TV, FFZ, Chatterino, Homies and the other badge services, plus StreamNook membership badges."
          help="Services turns individual badge services off. Lit means shown."
          control={
            <Toggle
              enabled={cd.show_third_party_badges}
              onChange={() => setDesign({ show_third_party_badges: !cd.show_third_party_badges })}
            />
          }
        >
          {cd.show_third_party_badges && (
            <SubControls>
              <SubControl
                title="Services"
                onReset={resetFor(['chat_design.hidden_badge_providers', []])}
                stacked
                control={
                  <div className="flex flex-wrap gap-1.5">
                    {BADGE_PROVIDERS.map((p) => {
                      const on = !badgeProviderHidden(p.id);
                      return (
                        <button
                          key={p.id}
                          type="button"
                          aria-pressed={on}
                          onClick={() => toggleBadgeProvider(p.id)}
                          className={`px-2.5 py-1 rounded-full text-[12px] font-medium transition-colors ${
                            on ? 'chrome-glaze chrome-glaze--flat chrome-glaze--control text-textPrimary' : 'glass-button-static text-textMuted'
                          }`}
                        >
                          {p.label}
                        </button>
                      );
                    })}
                  </div>
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="Profile pictures"
          onReset={resetFor(['chat_design.show_avatars', true])}
          description="On platforms that send one (YouTube, TikTok), the chatter's picture leads their message."
          control={<Toggle enabled={cd.show_avatars} onChange={() => setDesign({ show_avatars: !cd.show_avatars })} />}
        />

        <SettingsRow
          title="@ prefix"
          onReset={resetFor(['chat_design.show_at_sign', false])}
          description="Writes every name as @name."
          control={<Toggle enabled={cd.show_at_sign} onChange={() => setDesign({ show_at_sign: !cd.show_at_sign })} />}
        />

        <SettingsRow
          title="Paint shadows"
          onReset={resetFor(['cosmetics.paint_shadows', 'all'])}
          description="Some paints stack several drop shadows for readability; keep them all, just one, or none if names look too noisy."
        >
          <SegmentedSelect<'all' | 'one' | 'none'>
            value={(settings.cosmetics?.paint_shadows ?? 'all') as 'all' | 'one' | 'none'}
            onChange={(value) => setCosmetics({ paint_shadows: value })}
            options={[
              { value: 'all', label: 'All' },
              { value: 'one', label: 'One' },
              { value: 'none', label: 'None' },
            ]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Mentions & Replies"
        description="How a message that mentions you, and a reply, stand out, and how an @name reads in chat."
      >
        <div className="pt-3">
          <MentionPreview design={cd} look={{ ...mentionLook(cd), ...mentionHover }} />
        </div>

        <SettingsRow
          title="Mention sound"
          onReset={resetFor(['chat_design.mention_sound', ''])}
          description="Plays when someone @s you in any chat you have open."
          help="At most once every few seconds per chat, and never in streamer mode. Add your own sounds under Custom Sounds."
        >
          <Dropdown
            value={cd.mention_sound ?? ''}
            onChange={(v) => setDesign({ mention_sound: v })}
            className="w-full"
            ariaLabel="Mention sound"
            options={mentionSoundOptions}
          />
          {cd.mention_sound && (
            <SubControls>
              <SubControl
                title="Volume"
                onReset={resetFor(['chat_design.mention_sound_volume', 100])}
                control={
                  <SoundVolume
                    value={cd.mention_sound_volume ?? 100}
                    onChange={(mention_sound_volume) => setDesign({ mention_sound_volume })}
                    sound={cd.mention_sound}
                    label="Mention sound volume"
                  />
                }
              />
              <SubControl
                title="Replies to you"
                onReset={resetFor(['chat_design.mention_sound_replies', true])}
                control={
                  <Toggle
                    enabled={cd.mention_sound_replies ?? true}
                    onChange={() => setDesign({ mention_sound_replies: !(cd.mention_sound_replies ?? true) })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="@name style"
          description="How an @mention reads in chat. Hover an option to see it in the preview."
          help="A /me message is italic as a whole: Like the message keeps a mention in it italic too, Never keeps it upright."
        >
          <SubControls>
            <SubControl
              title="Weight"
              onReset={resetFor(['chat_design.mention_weight', 'medium'])}
              control={
                <SegmentedSelect<MentionWeight>
                  value={mentionLook(cd).weight}
                  onChange={(mention_weight) => setDesign({ mention_weight })}
                  onPreview={hoverMention('weight')}
                  options={[
                    { value: 'regular', label: 'Regular' },
                    { value: 'medium', label: 'Medium' },
                    { value: 'bold', label: 'Bold' },
                  ]}
                />
              }
            />
            <SubControl
              title="Italic"
              onReset={resetFor(['chat_design.mention_italic', 'inherit'])}
              control={
                <SegmentedSelect<MentionItalic>
                  value={mentionLook(cd).italic}
                  onChange={(mention_italic) => setDesign({ mention_italic })}
                  onPreview={hoverMention('italic')}
                  options={[
                    { value: 'inherit', label: 'Like the message' },
                    { value: 'never', label: 'Never' },
                    { value: 'always', label: 'Always' },
                  ]}
                />
              }
            />
            <SubControl
              title="Shape"
              onReset={resetFor(['chat_design.mention_style', 'plain'])}
              control={
                <SegmentedSelect<MentionShape>
                  value={mentionLook(cd).shape}
                  onChange={(mention_style) => setDesign({ mention_style })}
                  onPreview={hoverMention('shape')}
                  options={[
                    { value: 'plain', label: 'Plain' },
                    { value: 'pill', label: 'Pill' },
                  ]}
                />
              }
            />
            <SubControl
              title="Color"
              onReset={resetFor(['chat_design.mention_text_color', ''])}
              control={
                <div className="flex items-center gap-2">
                  {cd.mention_text_color && (
                    <ColorSwatch
                      value={cd.mention_text_color}
                      onChange={(color) => setDesign({ mention_text_color: color })}
                      tooltip="@mention color"
                    />
                  )}
                  <SegmentedSelect<'name' | 'fixed'>
                    value={cd.mention_text_color ? 'fixed' : 'name'}
                    onChange={(mode) => setDesign({ mention_text_color: mode === 'fixed' ? cd.mention_text_color || '#bf94ff' : '' })}
                    options={[
                      { value: 'name', label: 'Their color' },
                      { value: 'fixed', label: 'One color' },
                    ]}
                  />
                </div>
              }
            />
          </SubControls>
        </SettingsRow>

        <SettingsRow
          title="Highlight"
          onReset={resetFor(['chat_design.mention_color', '#ff4444'])}
          description="The color that marks a message mentioning or replying to you."
          help="Flash briefly lights the message up as it lands, so you spot it in a fast chat. The color edge stays either way."
        >
          <ColorSwatch
            value={cd.mention_color ?? '#ff4444'}
            onChange={(color) => setDesign({ mention_color: color })}
            tooltip="Mention color"
          />
          <SubControls>
            <SubControl
              title="Flash"
              onReset={resetFor(['chat_design.mention_animation', true])}
              control={
                <Toggle
                  enabled={cd.mention_animation ?? true}
                  onChange={() => setDesign({ mention_animation: !(cd.mention_animation ?? true) })}
                />
              }
            />
          </SubControls>
        </SettingsRow>

        {IS_MOBILE && (
          <SettingsRow
            title="Vibration"
            description="A short buzz when a message says your name, on top of the highlight."
            control={<Toggle enabled={mentionHaptic} onChange={() => setMentionHaptic(!mentionHaptic)} />}
          />
        )}

        <SettingsRow
          title="7TV paint"
          onReset={resetFor(['chat_design.paint_mentions_in_body', true])}
          description="Draws a mentioned name in that person's 7TV paint instead of a flat color."
          help="Off shows mentions in the chatter's plain name color."
          control={
            <Toggle
              enabled={cd.paint_mentions_in_body}
              onChange={() => setDesign({ paint_mentions_in_body: !cd.paint_mentions_in_body })}
            />
          }
        />

        <SettingsRow
          title="Thread color"
          onReset={resetFor(['chat_design.reply_color', '#ff6b6b'])}
          description="The color that marks replies in a thread."
        >
          <ColorSwatch
            value={cd.reply_color ?? '#ff6b6b'}
            onChange={(color) => setDesign({ reply_color: color })}
            tooltip="Reply thread color"
          />
        </SettingsRow>

        <SettingsRow
          title="Context"
          onReset={resetFor(['chat_design.reply_style', 'full'])}
          description="How a reply shows the message it answers: a context line above it, an @name at the start, or nothing."
        >
          <SegmentedSelect<'full' | 'mention' | 'off'>
            value={cd.reply_style}
            onChange={(reply_style) => setDesign({ reply_style })}
            options={[
              { value: 'full', label: 'Context line' },
              { value: 'mention', label: '@name' },
              { value: 'off', label: 'Off' },
            ]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Links"
        description="How links in chat look, and which sites expand into a preview card on their own."
      >
        <SettingsRow
          title="Style"
          onReset={resetFor(['chat_design.link_previews', true], ['chat_design.link_preview_keep_link', false])}
          description="Off keeps links as plain text, Card + Link adds a preview card under the link, and Clean shows only the card."
          help="In Clean, hover the card to see where it goes. StreamNook fetches the page from your PC to build the card, so the site sees a visit from you. Trusted sites expand on their own; every other link shows a Load preview button, and the shield on that button trusts the site from chat. Popular sites are trusted out of the box."
        >
          <SegmentedSelect<'off' | 'with_link' | 'clean'>
            value={
              !cd.link_previews ? 'off' : cd.link_preview_keep_link ? 'with_link' : 'clean'
            }
            onChange={(mode) => {
              if (mode === 'off') {
                setDesign({ link_previews: false });
              } else {
                setDesign({ link_previews: true, link_preview_keep_link: mode === 'with_link' });
              }
            }}
            options={[
              { value: 'off', label: 'Off' },
              { value: 'with_link', label: 'Card + Link' },
              { value: 'clean', label: 'Clean' },
            ]}
          />
          {cd.link_previews && (
            <SubControls>
              <SubControl
                title="Trusted sites"
                stacked
                control={
                  <TrustedSourcesEditor
                    domains={cd.link_preview_trusted_domains}
                    onChange={(next) => setDesign({ link_preview_trusted_domains: next })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>

        <SettingsRow
          title="Short links"
          onReset={resetFor(['chat_design.shorten_links', true])}
          description="Shows each link as a compact label, the site plus a short path, instead of the full raw URL."
          help="The full link still opens on click and shows on hover."
          control={
            <Toggle
              enabled={cd.shorten_links ?? true}
              onChange={() => setDesign({ shorten_links: !(cd.shorten_links ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Color"
          onReset={resetFor(['chat_design.link_color', ''])}
          description="The color of links in chat. Leave it on the default to follow the theme."
        >
          <ColorSwatch
            value={cd.link_color || '#8ab4ff'}
            onChange={(color) => setDesign({ link_color: color })}
            tooltip="Link color"
          />
        </SettingsRow>

        <SettingsRow
          title="Underline"
          onReset={resetFor(['chat_design.link_underline', true])}
          description="Off leaves links colored but not underlined."
          control={<Toggle enabled={cd.link_underline} onChange={() => setDesign({ link_underline: !cd.link_underline })} />}
        />
      </SettingsSection>

      <SettingsSection
        label="Emotes"
        description="How big emotes are, how they animate, and how much they grow when you hover one."
      >
        <SettingsRow
          title="Emoji style"
          onReset={resetFor(['chat_design.emoji_style', 'apple'])}
          description="Which set draws the emoji in messages. System uses your device's own."
        >
          <Dropdown<'system' | 'apple' | 'google' | 'twitter' | 'facebook'>
            value={cd.emoji_style}
            onChange={(emoji_style) => setDesign({ emoji_style })}
            className="w-full"
            ariaLabel="Emoji style"
            options={[
              { value: 'apple', label: 'Apple' },
              { value: 'google', label: 'Google' },
              { value: 'twitter', label: 'Twitter' },
              { value: 'facebook', label: 'Facebook' },
              { value: 'system', label: 'System' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Personal emotes"
          onReset={resetFor(['chat_design.show_personal_emotes', true])}
          description="Emotes from a chatter's own 7TV personal set. Off shows the text they typed instead."
          control={
            <Toggle
              enabled={cd.show_personal_emotes}
              onChange={() => setDesign({ show_personal_emotes: !cd.show_personal_emotes })}
            />
          }
        />

        <SettingsRow
          title="Animation"
          onReset={resetFor(['chat_design.animate_emotes', 'always'])}
          description={
            IS_MOBILE
              ? 'Play animated emotes, or show only their first frame. Never is the lightest on the GPU in a fast chat.'
              : 'Play animated emotes always, only while you hover a message, or never (first frame). Never is the lightest on the GPU in a fast chat.'
          }
        >
          {/* No hover on a touch screen: a saved "On hover" already behaves as
              Never there (ChatMessage skips touch pointers), so the phone shows
              it as Never and only writes when the viewer picks something. */}
          <SegmentedSelect<'always' | 'hover' | 'never'>
            value={IS_MOBILE && cd.animate_emotes === 'hover' ? 'never' : (cd.animate_emotes ?? 'always')}
            onChange={(animate_emotes) => setDesign({ animate_emotes })}
            options={
              IS_MOBILE
                ? [
                    { value: 'always', label: 'Always' },
                    { value: 'never', label: 'Never' },
                  ]
                : [
                    { value: 'always', label: 'Always' },
                    { value: 'hover', label: 'On hover' },
                    { value: 'never', label: 'Never' },
                  ]
            }
          />
        </SettingsRow>

        <SettingsRow
          title="GIFs"
          onReset={resetFor(['chat_design.show_chat_gifs', true])}
          description="Twitch lets Tier 2 and Tier 3 subscribers post GIFs. Off swaps each one for a small chip you can click to reveal."
          help={
            IS_MOBILE
              ? 'GIFs also follow Animation: Never shows the chip instead.'
              : 'GIFs also follow Animation: Never shows the chip, On hover plays them while you hover the message.'
          }
          control={
            <Toggle
              enabled={cd.show_chat_gifs ?? true}
              onChange={() => setDesign({ show_chat_gifs: !(cd.show_chat_gifs ?? true) })}
            />
          }
        />

        <SettingsRow
          title={`Size: ${(cd.emote_scale ?? 1).toFixed(2)}x`}
          onReset={resetFor(['chat_design.emote_scale', 1])}
          description="Scales emotes in chat relative to the text, with 1.00x being the default size."
        >
          <input
            type="range"
            min="0.5"
            max="3"
            step="0.05"
            value={cd.emote_scale ?? 1}
            onChange={(e) => setDesign({ emote_scale: parseFloat(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        {/* Desktop only: a phone never shows emote tooltips (Tooltip.tsx
            returns early on IS_MOBILE). */}
        {!IS_MOBILE && (
          <SettingsRow
            title={`Hover size: ${(HOVER_SIZE_OPTIONS.find((o) => o.px === cd.emote_hover_size) ?? HOVER_SIZE_OPTIONS[1]).label}`}
            onReset={resetFor(['chat_design.emote_hover_size', 96])}
            description={
              cd.compact_emote_tooltips
                ? 'Off while Compact is on, since that replaces the hover card with just the emote name.'
                : 'How large an emote grows when you hover it, in chat and in the emote menu.'
            }
            help='Hover the sample below to try the chosen size. The size of emotes in the message still follows Size above. Compact shows just the emote name on hover instead of the card and its "Right-click to copy" hint.'
          >
            <div className={`space-y-3 ${cd.compact_emote_tooltips ? 'pointer-events-none opacity-50' : ''}`}>
              <SegmentedSelect<HoverSizeKey>
                value={(HOVER_SIZE_OPTIONS.find((o) => o.px === cd.emote_hover_size) ?? HOVER_SIZE_OPTIONS[1]).value}
                options={HOVER_SIZE_OPTIONS.map((o) => ({ value: o.value, label: o.label }))}
                onChange={(v) => {
                  const opt = HOVER_SIZE_OPTIONS.find((o) => o.value === v) ?? HOVER_SIZE_OPTIONS[1];
                  setDesign({ emote_hover_size: opt.px });
                }}
              />
              <EmoteHoverDemo hoverSize={cd.emote_hover_size} emoteScale={cd.emote_scale} />
            </div>
            <SubControls>
              <SubControl
                title="Compact"
                onReset={resetFor(['chat_design.compact_emote_tooltips', false])}
                control={
                  <Toggle
                    enabled={cd.compact_emote_tooltips}
                    onChange={() => setDesign({ compact_emote_tooltips: !cd.compact_emote_tooltips })}
                  />
                }
              />
            </SubControls>
          </SettingsRow>
        )}

        <SettingsRow
          title={`Spacing: ${(cd.emote_margin ?? 0.125).toFixed(3)}rem`}
          onReset={resetFor(['chat_design.emote_margin', 0.125])}
          description="Space on each side of an emote; go negative to let neighboring emotes overlap."
        >
          <input
            type="range"
            min="-0.5"
            max="0.5"
            step="0.025"
            value={cd.emote_margin ?? 0.125}
            onChange={(e) => setDesign({ emote_margin: parseFloat(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title="7TV notices"
          onReset={resetFor(['chat_design.seventv_emote_notices', true])}
          description="Shows a chat notice when a mod adds, removes, or renames a 7TV emote in the channel."
          help="The new emote is usable right away either way."
          control={
            <Toggle
              enabled={cd.seventv_emote_notices ?? true}
              onChange={() => setDesign({ seventv_emote_notices: !(cd.seventv_emote_notices ?? true) })}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Emote Effects"
        description="Emotes that change the emote beside them, and Twitch's giant power-up emotes."
      >
        <SettingsRow
          title="FFZ effects"
          onReset={resetFor(['chat_design.ffz_emote_effects', true])}
          description="Applies FrankerFaceZ modifiers (wide, flips, rainbow, shake) to the emote before them, the way FFZ does."
          help="Off shows modifier emotes as plain overlay emotes."
          control={
            <Toggle
              enabled={cd.ffz_emote_effects}
              onChange={() => setDesign({ ffz_emote_effects: !cd.ffz_emote_effects })}
            />
          }
        />

        <SettingsRow
          title="BetterTTV modifiers"
          onReset={resetFor(['chat_design.bttv_emote_modifiers', true])}
          description="Applies BetterTTV modifiers (w! wide, h! and v! flips, c! cursed, p! party, s! shake) to the emote after them, the way BetterTTV does."
          help="Off shows the modifiers as plain emotes."
          control={
            <Toggle
              enabled={cd.bttv_emote_modifiers}
              onChange={() => setDesign({ bttv_emote_modifiers: !cd.bttv_emote_modifiers })}
            />
          }
        />

        <SettingsRow
          title="Giant emotes"
          onReset={resetFor(['chat_design.giant_emotes', true])}
          description={'Draws the last emote of a "Gigantify an Emote" power-up message at 4x below the message, like Twitch does.'}
          help="Off shows the emote inline at its normal size. Position puts it under the message on the left, centered or on the right, or keeps it in the text at its normal size."
          control={
            <Toggle
              enabled={cd.giant_emotes}
              onChange={() => setDesign({ giant_emotes: !cd.giant_emotes })}
            />
          }
        >
          {cd.giant_emotes && (
            <SubControls>
              <SubControl
                title="Position"
                onReset={resetFor(['chat_design.giant_emote_align', 'center'])}
                control={
                  <SegmentedSelect<'left' | 'center' | 'right' | 'inline'>
                    value={cd.giant_emote_align}
                    onChange={(giant_emote_align) => setDesign({ giant_emote_align })}
                    options={[
                      { value: 'left', label: 'Left' },
                      { value: 'center', label: 'Center' },
                      { value: 'right', label: 'Right' },
                      { value: 'inline', label: 'In the text' },
                    ]}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Chat Input"
        description="Small conveniences in the box where you type, and which buttons sit around it."
      >
        <SettingsRow
          title="Duplicate sends"
          onReset={resetFor(['chat_input.bypass_duplicate', false])}
          description="Lets you send the same message twice in a row by adding an invisible character, so Twitch does not reject the second send."
          help="Twitch normally blocks identical messages sent back to back. Handy for repeating an emote."
          control={
            <Toggle
              enabled={settings.chat_input?.bypass_duplicate ?? false}
              onChange={() => setInput({ bypass_duplicate: !(settings.chat_input?.bypass_duplicate ?? false) })}
            />
          }
        />
        {/* Desktop only: there is no Ctrl to hold on a phone keyboard. */}
        {!IS_MOBILE && (
          <SettingsRow
            title="Ctrl+Enter"
            onReset={resetFor(['chat_input.quick_send', false])}
            description="Sends the message and keeps the text in the box, so you can send it again straight away."
            help="Plain Enter still sends and clears the box as normal."
            control={
              <Toggle
                enabled={settings.chat_input?.quick_send ?? false}
                onChange={() => setInput({ quick_send: !(settings.chat_input?.quick_send ?? false) })}
              />
            }
          />
        )}
        <SettingsRow
          title="Spellcheck"
          onReset={resetFor(['chat_input.spellcheck_enabled', true])}
          description="Underlines misspelled words as you type and offers corrections when you right-click one. Emotes, chatters, commands and links are left alone."
          help="Dictionary holds the words you have taught it, added when you pick Add to dictionary on a word in chat. Remove one here after a mis-click."
          control={
            <Toggle
              enabled={settings.chat_input?.spellcheck_enabled ?? true}
              onChange={() => setInput({ spellcheck_enabled: !(settings.chat_input?.spellcheck_enabled ?? true) })}
            />
          }
        >
          {(settings.chat_input?.spellcheck_enabled ?? true) && (
            <SubControls>
              <SubControl title="Dictionary" stacked control={<SpellcheckDictionary />} />
            </SubControls>
          )}
        </SettingsRow>
        <SettingsRow
          title="Hide placeholder"
          onReset={resetFor(['chat_input.hide_placeholder', false])}
          description="Leaves the message box empty instead of prompting you to send a message. Notices you can act on, like read-only or subscriber-only mode, still show."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_placeholder ?? false}
              onChange={() => setInput({ hide_placeholder: !(settings.chat_input?.hide_placeholder ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Hide command button"
          onReset={resetFor(['chat_input.hide_command_button', false])}
          description="Removes the slash button from inside the message box. Typing / still opens the quick command list."
          help="The command button opens a browsable menu of every command you can run here, with what each one does and examples you can click into the box. It also has a larger view for reading comfortably."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_command_button ?? false}
              onChange={() => setInput({ hide_command_button: !(settings.chat_input?.hide_command_button ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Hide emote button"
          onReset={resetFor(['chat_input.hide_emote_button', false])}
          description="Removes the smiley from inside the message box. The emote picker is still reachable from its keyboard shortcut and from tab completion."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_emote_button ?? false}
              onChange={() => setInput({ hide_emote_button: !(settings.chat_input?.hide_emote_button ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Points balance"
          onReset={resetFor(['chat_input.hide_points_balance', false], ['chat_input.show_points_balance_inline', false])}
          description="Always shows your channel points beside the button next to the message box. Hidden removes the button, which still comes back whenever a bonus chest is waiting."
          control={
            <SegmentedSelect<'always' | 'hover' | 'hidden'>
              value={
                settings.chat_input?.hide_points_balance
                  ? 'hidden'
                  : settings.chat_input?.show_points_balance_inline
                    ? 'always'
                    : 'hover'
              }
              onChange={(mode) =>
                setInput({
                  hide_points_balance: mode === 'hidden',
                  show_points_balance_inline: mode === 'always',
                })
              }
              options={[
                { value: 'always', label: 'Always' },
                { value: 'hover', label: 'On hover' },
                { value: 'hidden', label: 'Hidden' },
              ]}
            />
          }
        />

      </SettingsSection>

      {/* Desktop only: the paste-to-upload flow lives in the desktop composer;
          the phone composer has no uploader, so this section would configure
          nothing there. */}
      {!IS_MOBILE && <ImageUploadSettings />}

      {/* Shown on both now. It used to be hidden on the phone because the
          feature was Tab-driven and there is no Tab key; the phone composer
          reaches the same suggestions by swiping a strip above the input. The
          settings themselves were always shared, and hiding the section left
          them searchable but unreachable.

          Only the wording differs, and it differs through a ternary rather than
          a rewrite: these strings are mirrored in the settings search index and
          the command palette, neither of which is platform-aware, so changing
          the desktop copy here would silently desync three files. */}
      <SettingsSection
        label="Tab Completion"
        description={
          IS_MOBILE
            ? 'Type part of an emote name in chat to see matching emotes above the input. Swipe the strip to see more, tap one to use it.'
            : 'Press Tab to complete the emote you are typing, or type : and two letters to see every emote you can use.'
        }
        id="settings-section-emote-tab-completion"
      >
        <SettingsRow
          title="Emotes"
          onReset={resetFor(['chat_input.emote_tab_complete_enabled', true])}
          description={
            IS_MOBILE
              ? 'Suggest matching emotes as you type.'
              : 'Press Tab to complete the emote you are typing, in a carousel or a list.'
          }
          help={
            IS_MOBILE
              ? 'Matching: Starts With needs the emote to begin with what you typed; Contains matches it anywhere in the name.'
              : 'Style: Carousel puts the best match straight into your message, and each Tab after that swaps in the next one (Shift+Tab goes back; on an empty spot it starts with your favorites and this channel\'s emotes). List opens every emote you can use with where it comes from; Tab or the arrows move, Enter inserts, Esc closes. Matching: Starts With needs the emote to begin with what you typed, Contains matches anywhere in the name. The : list always looks inside names, so :love finds vulpLove.'
          }
          control={
            <Toggle
              enabled={settings.chat_input?.emote_tab_complete_enabled ?? true}
              onChange={() =>
                setInput({
                  emote_tab_complete_enabled: !(settings.chat_input?.emote_tab_complete_enabled ?? true),
                })
              }
            />
          }
        >
          {(settings.chat_input?.emote_tab_complete_enabled ?? true) && (
            <SubControls>
              {!IS_MOBILE && (
                <SubControl
                  title="Style"
                  onReset={resetFor(['chat_input.emote_tab_style', 'carousel'])}
                  control={
                    <SegmentedSelect<'carousel' | 'list'>
                      value={settings.chat_input?.emote_tab_style ?? 'carousel'}
                      options={[
                        { value: 'carousel', label: 'Carousel' },
                        { value: 'list', label: 'List' },
                      ]}
                      onChange={(v) => setInput({ emote_tab_style: v })}
                    />
                  }
                />
              )}
              <SubControl
                title="Matching"
                onReset={resetFor(['chat_input.emote_tab_complete_match_mode', 'starts_with'])}
                control={
                  <SegmentedSelect<'starts_with' | 'includes'>
                    value={settings.chat_input?.emote_tab_complete_match_mode ?? 'starts_with'}
                    options={[
                      { value: 'starts_with', label: 'Starts With' },
                      { value: 'includes', label: 'Contains' },
                    ]}
                    onChange={(v) => setInput({ emote_tab_complete_match_mode: v })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
        {!IS_MOBILE && (
          <SettingsRow
            title="Colon list"
            onReset={resetFor(['chat_input.emote_colon_search_enabled', true])}
            description="Type a colon and two letters to see every emote you can use and where it comes from."
            control={
              <Toggle
                enabled={settings.chat_input?.emote_colon_search_enabled ?? true}
                onChange={() =>
                  setInput({
                    emote_colon_search_enabled: !(settings.chat_input?.emote_colon_search_enabled ?? true),
                  })
                }
              />
            }
          />
        )}

        <SettingsRow
          title="Chatter names"
          onReset={resetFor(['chat_input.emote_tab_complete_include_chatters', true])}
          description={
            IS_MOBILE
              ? 'Also suggest display names of users currently in chat.'
              : 'Also cycles through the names of people currently in chat.'
          }
          control={
            <Toggle
              enabled={settings.chat_input?.emote_tab_complete_include_chatters ?? true}
              onChange={() =>
                setInput({
                  emote_tab_complete_include_chatters: !(settings.chat_input?.emote_tab_complete_include_chatters ?? true),
                })
              }
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Channel Points"
        description="Bonus chest pickup on the channel you are watching."
      >
        <SettingsRow
          title="Bonus chests"
          onReset={resetFor(['auto_claim_points_watching', true])}
          description="Collects the bonus chest on the stream you are watching the moment it appears."
          help="When this is off, a claim button appears on the points icon so you can grab it yourself. Claiming on channels you are not watching is a separate opt-in plugin."
          control={
            <Toggle
              enabled={settings.auto_claim_points_watching ?? true}
              onChange={() =>
                updateSettings({
                  ...settings,
                  auto_claim_points_watching: !(settings.auto_claim_points_watching ?? true),
                })
              }
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        label="Chat Behavior"
        description="Deleted messages, shared chat, scrolling, and how much chat is kept."
      >
        <SettingsRow
          title="Deleted messages"
          onReset={resetFor(['chat_design.deleted_message_style', 'strikethrough'])}
          description="How a message looks once a moderator deletes it or times out or bans its sender."
        >
          <div className="space-y-3">
            <SegmentedSelect<DeletedMessageStyle>
              value={cd.deleted_message_style as DeletedMessageStyle}
              onChange={(value) => setDesign({ deleted_message_style: value })}
              onPreview={setDeletedHover}
              options={[
                { value: 'strikethrough', label: 'Strikethrough' },
                { value: 'dimmed', label: 'Dimmed' },
                { value: 'italic', label: 'Italic' },
                { value: 'keep', label: 'Keep' },
                { value: 'hidden', label: 'Hidden' },
              ]}
            />
            <DeletedMessagePreview
              design={cd}
              style={deletedHover ?? (cd.deleted_message_style as DeletedMessageStyle)}
            />
          </div>
        </SettingsRow>

        <SettingsRow
          title="Hide shared chat"
          onReset={resetFor(['chat_design.hide_shared_chat', false])}
          description="Hides messages that came from the other channel in a Twitch Shared Chat, so you only see this channel's own chatters."
          control={
            <Toggle
              enabled={cd.hide_shared_chat}
              onChange={() => setDesign({ hide_shared_chat: !cd.hide_shared_chat })}
            />
          }
        />

        <SettingsRow
          title="Smooth resume"
          onReset={resetFor(['chat_render.smooth_scroll_on_resume', true])}
          description="Animates the scroll back to the bottom when you click Resume; auto-scroll for new messages stays instant."
          control={
            <Toggle
              enabled={settings.chat_render?.smooth_scroll_on_resume ?? true}
              onChange={() =>
                setRender({ smooth_scroll_on_resume: !(settings.chat_render?.smooth_scroll_on_resume ?? true) })
              }
            />
          }
        />

        <SettingsRow
          title={`Scrollback: ${Math.min(IS_MOBILE ? 300 : 1000, settings.chat_render?.message_buffer_cap ?? 100)} messages`}
          onReset={resetFor(['chat_render.message_buffer_cap', 100])}
          description={
            IS_MOBILE
              ? 'How many messages each chat keeps to scroll back through. Phones stop at 300: more than that costs memory and smoothness with nothing extra to see.'
              : 'How many messages each chat keeps on screen to scroll back through; more history uses more memory.'
          }
        >
          <input
            type="range"
            min="50"
            max={IS_MOBILE ? 300 : 1000}
            step="10"
            value={Math.min(IS_MOBILE ? 300 : 1000, settings.chat_render?.message_buffer_cap ?? 100)}
            onChange={(e) => setRender({ message_buffer_cap: parseInt(e.target.value, 10) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Docked Chats"
        description="Chats you keep open beside the stream, and how you switch between them."
      >
        <SettingsRow
          title="Menu style"
          onReset={resetFor(['chat_design.chat_dock_switcher', 'menu'])}
          description="A list that opens from the chat name, or a row of tabs under the chat header so each chat is one click away."
        >
          <SegmentedSelect<'menu' | 'tabs'>
            value={cd.chat_dock_switcher ?? 'menu'}
            onChange={(chat_dock_switcher) => setDesign({ chat_dock_switcher })}
            options={[
              { value: 'menu', label: 'List' },
              { value: 'tabs', label: 'Tabs' },
            ]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-repeated-messages"
        label="Repeated Messages"
        description="When several people post the same thing at once, fold the run into one row with a count instead of repeating it down the whole chat."
      >
        <SettingsRow
          title="Mode"
          onReset={resetFor(['message_repeat.mode', 'off'])}
          description={
            repeatMode === 'collapse'
              ? 'Keeps the first one and counts the rest onto it.'
              : repeatMode === 'label'
                ? 'Leaves every message in chat and just numbers them, so nothing is hidden.'
                : 'Repeats are left completely alone.'
          }
          help={'Match: "Nearly the same" ignores capitals, extra spaces and trailing punctuation, so "LULW!!" joins "lulw". Minimum is how many copies it takes before the counter appears. Window is how long a run stays open; after that the next copy starts a fresh run. Counter color is the little x12. Mod & VIP exemption keeps mods, VIPs and the streamer on their own rows. Moderated channels turns folding off wherever you are a mod, so a hidden copy is never a message you needed to action.'}
        >
          <SegmentedSelect<RepeatDisplayMode>
            value={repeatMode}
            onChange={(mode) => setRepeat({ mode })}
            options={[
              { value: 'collapse', label: 'Fold into one' },
              { value: 'label', label: 'Just count them' },
              { value: 'off', label: 'Off' },
            ]}
          />
          {repeatMode !== 'off' && (
            <SubControls>
              <SubControl
                title="Match"
                onReset={resetFor(['message_repeat.match', 'normalized'])}
                control={
                  <SegmentedSelect<RepeatMatchMode>
                    value={rp?.match ?? 'normalized'}
                    onChange={(match) => setRepeat({ match })}
                    options={[
                      { value: 'normalized', label: 'Nearly the same' },
                      { value: 'exact', label: 'Exactly the same' },
                    ]}
                  />
                }
              />
              <SubControl
                title="Minimum"
                onReset={resetFor(['message_repeat.threshold', 2])}
                control={
                  <InlineSlider
                    value={repeatThreshold}
                    min={2}
                    max={10}
                    label="Copies before the counter shows"
                    format={(v) => `${v} copies`}
                    onChange={(threshold) => setRepeat({ threshold })}
                  />
                }
              />
              <SubControl
                title="Window"
                onReset={resetFor(['message_repeat.window_seconds', 60])}
                control={
                  <InlineSlider
                    value={repeatWindow}
                    min={10}
                    max={300}
                    step={5}
                    label="Seconds a run stays open"
                    format={(v) => `${v}s`}
                    onChange={(window_seconds) => setRepeat({ window_seconds })}
                  />
                }
              />
              <SubControl
                title="Counter color"
                onReset={resetFor(['message_repeat.color', REPEAT_DEFAULT_COLOR])}
                control={
                  <ColorSwatch
                    value={rp?.color || REPEAT_DEFAULT_COLOR}
                    onChange={(color) => setRepeat({ color })}
                    tooltip="Pick the counter color"
                  />
                }
              />
              <SubControl
                title="Mod & VIP exemption"
                onReset={resetFor(['message_repeat.exempt_privileged', true])}
                control={
                  <Toggle
                    enabled={rp?.exempt_privileged !== false}
                    onChange={() => setRepeat({ exempt_privileged: rp?.exempt_privileged === false })}
                  />
                }
              />
              <SubControl
                title="Moderated channels"
                onReset={resetFor(['message_repeat.keep_all_when_moderator', true])}
                control={
                  <Toggle
                    enabled={rp?.keep_all_when_moderator !== false}
                    onChange={() => setRepeat({ keep_all_when_moderator: rp?.keep_all_when_moderator === false })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        id="settings-section-chat-filters"
        label="Hidden Messages"
        description="Keep chosen users, bots, commands and phrases out of your chat. Only affects what you see; nothing is sent to the platform, and your own messages are never hidden."
      >
        <SettingsRow
          title="Known bots"
          onReset={resetFor(['chat_filters.hide_bots', false])}
          description="Hides StreamElements, Nightbot, Moobot and the other well-known chat bots, in every channel."
          control={
            <Toggle
              enabled={cfs?.hide_bots ?? false}
              onChange={() => setChatFilters({ hide_bots: !(cfs?.hide_bots ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Everywhere"
          description="Messages from these names never appear, on any platform. Add someone here, or from their user card in chat."
        >
          <HiddenNameEditor
            names={cfs?.hidden_users ?? []}
            onAdd={(n) => setHidden(n, 'global', true)}
            onRemove={(n) => setHidden(n, 'global', false)}
          />
        </SettingsRow>
        {perChannelHidden.length > 0 && (
          <SettingsRow
            title="Per channel"
            description="Added from user cards while watching. Removing a name shows their messages again in that channel."
          >
            <div className="flex flex-col gap-2 w-full">
              {perChannelHidden.map((entry) => (
                <div key={entry.key} className="flex flex-wrap items-center gap-1.5">
                  <span className="text-xs font-semibold text-textSecondary min-w-24">{entry.label}</span>
                  {entry.names.map((n) => (
                    <span key={n} className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full bg-surface text-xs text-textPrimary">
                      {n}
                      <button
                        aria-label={`Unhide ${n} in ${entry.label}`}
                        className="text-textSecondary hover:text-error"
                        onClick={() => setHidden(n, { provider: entry.pk.provider, channel: entry.pk.channel }, false)}
                      >
                        ×
                      </button>
                    </span>
                  ))}
                </div>
              ))}
            </div>
          </SettingsRow>
        )}
        <SettingsRow
          title="Bot commands"
          onReset={resetFor(['chat_filters.hide_commands', false])}
          description="Hides messages that are bot commands, so a chat full of !drops and !uptime reads as a chat."
          help="Patterns: a prefix hides every command starting with it; an exact pattern hides only that word at the start of a message. With no patterns, anything starting with ! is hidden. Your own messages are never hidden."
          control={
            <Toggle
              enabled={settings.chat_filters?.hide_commands ?? false}
              onChange={() => setCommandFilters(commandFilters, !(settings.chat_filters?.hide_commands ?? false))}
            />
          }
        >
          {(settings.chat_filters?.hide_commands ?? false) && (
            <SubControls>
              <SubControl
                title="Patterns"
                stacked
                control={
                  <div className="flex flex-col gap-2 w-full">
                    <div className="flex flex-wrap gap-1.5">
                      {commandFilters.length === 0 && (
                        <span className="text-[12px] text-textMuted">Using the default: anything starting with !</span>
                      )}
                      {commandFilters.map((f, i) => (
                        <button
                          key={`${f.mode}:${f.value}:${i}`}
                          type="button"
                          onClick={() => setCommandFilters(commandFilters.filter((_, j) => j !== i))}
                          title="Remove"
                          className="glass-button-static px-2.5 py-1 rounded-full text-[12px] text-textPrimary flex items-center gap-1.5"
                        >
                          <span className="font-mono">{f.value}</span>
                          <span className="text-textMuted">{f.mode === 'exact' ? 'exact' : 'prefix'}</span>
                          <X size={12} className="text-textMuted" />
                        </button>
                      ))}
                    </div>
                    <div className="flex items-center gap-2">
                      <input
                        type="text"
                        value={commandDraft}
                        onChange={(e) => setCommandDraft(e.target.value)}
                        placeholder="!"
                        maxLength={40}
                        className="glass-input flex-1 min-w-0 px-3 py-2 text-[13px] font-mono text-textPrimary placeholder:text-textMuted"
                        onKeyDown={(e) => {
                          if (e.key === 'Enter' && commandDraft.trim()) {
                            setCommandFilters([...commandFilters, { value: commandDraft.trim(), mode: commandMode }]);
                            setCommandDraft('');
                          }
                        }}
                      />
                      <SegmentedSelect<'prefix' | 'exact'>
                        value={commandMode}
                        onChange={setCommandMode}
                        options={[
                          { value: 'prefix', label: 'Prefix' },
                          { value: 'exact', label: 'Exact' },
                        ]}
                      />
                      <button
                        type="button"
                        disabled={!commandDraft.trim()}
                        onClick={() => {
                          setCommandFilters([...commandFilters, { value: commandDraft.trim(), mode: commandMode }]);
                          setCommandDraft('');
                        }}
                        className="glass-button px-3 py-2 text-[13px] font-medium text-textPrimary disabled:opacity-50"
                      >
                        Add
                      </button>
                    </div>
                  </div>
                }
              />
            </SubControls>
          )}
        </SettingsRow>
        <SettingsRow
          title="Ignored phrases"
          description="Never see messages that contain these words, in any chat. The sender is not told. Plain words work; turn on regex or whole-word per phrase when you need precision."
        >
          <IgnoredPhrasesSettings />
        </SettingsRow>
      </SettingsSection>

      {/* Desktop only: filters are applied from the funnel in a chat's header
          and search opens with Ctrl+F, and the phone chat pane has neither. */}
      {!IS_MOBILE && (
      <SettingsSection
        id="settings-section-chat-query"
        label="Filters & Search"
        description="Cut a busy chat down to what you care about, and find things you saw earlier. Filters live here; you apply one to a chat from the funnel in its header. Ctrl+F opens search in any chat."
      >
        <SettingsRow
          title="Filters"
          description="Mods only, subs only, mentions, links, or anything you can describe. Start from a preset, then choose the filter from the funnel in a chat's header. Switching filters never loses messages."
          help="Filters are short expressions like message.content contains 'giveaway' or author.subbed, combined with and, or and parentheses. Each message is checked once as it arrives, in the backend, so a filter costs nothing per window."
        >
          <SavedFiltersSettings />
        </SettingsRow>
        <SettingsRow
          title="Search depth"
          onReset={resetFor(['chat_query.history_cap', 1000])}
          description="How far back Ctrl+F search reaches: messages remembered per chat. Higher finds older messages; 1000 is roughly a megabyte per open chat."
          help="Kept in the backend, not in the chat view, so scrolling stays smooth no matter what you set. Range 200 to 5000."
        >
          <input
            type="number"
            min={200}
            max={5000}
            step={100}
            value={settings.chat_query?.history_cap ?? 1000}
            onChange={(e) => {
              const n = Math.max(200, Math.min(5000, Math.round(Number(e.target.value) || 1000)));
              updateSettings({
                ...settings,
                chat_query: { ...settings.chat_query, history_cap: n },
              });
            }}
            className="glass-input w-24 px-2.5 py-1.5 text-sm text-textPrimary"
          />
        </SettingsRow>
      </SettingsSection>
      )}

      {/* Desktop only, and the WHOLE section, not just the row: it holds one
          setting, so guarding the row alone would leave a titled section with
          nothing in it. `user_card_opens_messages` is read solely by
          UserProfileCard.tsx; the phone opens its own UserProfileSheet, which
          never consults it, so the toggle did nothing on Android. */}
      {!IS_MOBILE && (
      <SettingsSection
        id="settings-section-user-cards"
        label="User Cards"
        description="The card that opens when you click someone in chat."
      >
        <SettingsRow
          title="Messages first"
          onReset={resetFor(['chat_design.user_card_opens_messages', false])}
          description="Opens on the person's recent chat history straight away. Off opens the profile first, with their badges and stats. Either way the card switches between the two."
          control={
            <Toggle
              enabled={cd.user_card_opens_messages}
              onChange={() => setDesign({ user_card_opens_messages: !cd.user_card_opens_messages })}
            />
          }
        />
        {USER_CARD_ROWS.map(({ key, title, description }) => (
          <SettingsRow
            key={key}
            title={title}
            onReset={resetFor([`user_card.${key}`, key !== 'show_pronouns'])}
            description={description}
            control={
              <Toggle
                // Pronouns is the one opt-IN row (a third-party lookup); every
                // other row defaults to on.
                enabled={key === 'show_pronouns' ? settings.user_card?.show_pronouns === true : settings.user_card?.[key] !== false}
                onChange={() =>
                  setUserCard({
                    [key]:
                      key === 'show_pronouns'
                        ? settings.user_card?.show_pronouns !== true
                        : settings.user_card?.[key] === false,
                  })
                }
              />
            }
          />
        ))}
      </SettingsSection>
      )}

      {/* Desktop only. Writes .log files into a folder the user picks, and there
          is no user-visible folder to point at on Android. */}
      {!IS_MOBILE && (
      <SettingsSection
        label="Chat Logging"
        description="Keep a text copy of chat on your disk, for searching later or feeding another tool."
      >
        <SettingsRow
          title="Save logs"
          onReset={resetFor(['chat_logging.enabled', false])}
          description="Writes chat to plain text files as you watch: one folder per channel, one file per day."
          help="The files grow with the chat, so a busy channel adds up over weeks; delete old days from the folder any time. Folder is where they are written (Browse to pick your own; the arrow beside it puts the default back). Channels limits logging to the ones listed; empty logs every channel you open. Timestamps starts each line with the time it was sent. Events also logs subscriptions, raids, announcements, timeouts and deleted messages."
          control={
            <Toggle
              enabled={loggingEnabled}
              onChange={() => setLogging({ enabled: !loggingEnabled })}
            />
          }
        >
          {loggingEnabled && (
            <SubControls>
              <SubControl
                title="Folder"
                onReset={resetFor(['chat_logging.folder', ''])}
                stacked
                control={
                  <div className="flex items-center gap-2">
                    <div className="glass-input min-w-0 flex-1 truncate rounded-md px-3 py-1.5 text-[13px] text-textPrimary">
                      {logDir}
                    </div>
                    <button
                      type="button"
                      onClick={browseLogFolder}
                      className="glass-button-secondary flex-shrink-0 px-3 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary"
                    >
                      Browse
                    </button>
                    <button
                      type="button"
                      onClick={openLogFolder}
                      className="glass-button-secondary flex-shrink-0 px-3 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary"
                    >
                      Open
                    </button>
                  </div>
                }
              />
              <SubControl
                title="Channels"
                stacked
                control={
                  <PanelChannelList
                    value={logging.channels ?? []}
                    onChange={(channels) => setLogging({ channels })}
                  />
                }
              />
              <SubControl
                title="Timestamps"
                onReset={resetFor(['chat_logging.timestamps', true])}
                control={
                  <Toggle
                    enabled={logging.timestamps ?? true}
                    onChange={() => setLogging({ timestamps: !(logging.timestamps ?? true) })}
                  />
                }
              />
              <SubControl
                title="Events"
                onReset={resetFor(['chat_logging.include_events', true])}
                control={
                  <Toggle
                    enabled={logging.include_events ?? true}
                    onChange={() => setLogging({ include_events: !(logging.include_events ?? true) })}
                  />
                }
              />
            </SubControls>
          )}
        </SettingsRow>
      </SettingsSection>
      )}

      <HighlightAppearanceSettings />
      <CustomSoundsSettings />

      <HighlightPhrasesSettings />

      <BuiltInHighlightsSettings />

      <UserHighlightsSettings />

      <BadgeHighlightsSettings />

      <UserCommandsSettings />

      <RemindersSettings />

      <UserOverridesSettings />
    </div>
  );
};

export default ChatSettings;
