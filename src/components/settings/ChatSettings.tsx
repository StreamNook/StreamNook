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
import { Toggle } from '../ui/Toggle';
import { usePhonePrefs } from '../../mobile/phonePrefs';
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
import { withHiddenUser } from '../../utils/chatFilters';
import { parseKey } from '../../utils/providerKey';
import type { ProviderId } from '../../types/providers';

// Muted grey, so the counter reads as chrome rather than competing with the
// message. Matches --color-text-secondary in the default theme.
const REPEAT_DEFAULT_COLOR = '#8b8b8b';

// Rows the user card can show, in the order they appear on the card itself.
// Everything defaults to on; the toggle stores `false` to hide.
const USER_CARD_ROWS: { key: keyof UserCardSettings; title: string; description: string }[] = [
  { key: 'show_join_date', title: 'Joined Twitch', description: 'When the account was created.' },
  { key: 'show_followage', title: 'Following since', description: 'When they followed this channel, or that they are not following.' },
  { key: 'show_follows_count', title: 'Channels they follow', description: 'How many channels this person follows.' },
  { key: 'show_chatter_count', title: 'Chatters', description: "How many people are in this person's own chat right now." },
  { key: 'show_past_subscriber', title: 'Past subscriber', description: 'Total months subscribed, for people who are not subscribed now.' },
  { key: 'show_last_live', title: 'Last live', description: 'When they last streamed, if they ever have.' },
  { key: 'show_relative_time', title: 'Show "how long ago"', description: 'Adds a plain-English age next to dates, so "Mar 3, 2019" also reads "(6y ago)".' },
  { key: 'show_seventv_link', title: '7TV profile link', description: 'A 7TV chip next to their name that opens their 7TV profile in your browser.' },
  { key: 'show_pronouns', title: 'Pronouns', description: 'Their pronouns from pronouns.alejo.io, where chatters set them once for every chat client. One small request per person, cached for six hours. Off by default because it is a third-party lookup.' },
  { key: 'show_notes', title: 'Private notes', description: 'A note only you can see, kept with the user across renames. Handy for moderators.' },
];
import { useChatUserStore } from '../../stores/chatUserStore';
import { getUserCosmetics, computePaintStyle } from '../../services/seventvService';
import { StyledChatName, type NameSeparator, type NameStyle } from '../chat/StyledChatName';

// Native color swatch matching the mod-log Log Highlights control: clicking it
// opens the OS picker (always on top, unlike an in-app popover that can render
// behind later settings rows). Reset appears once the value leaves its default.
const ColorSwatch = ({
  value,
  defaultValue,
  onChange,
  tooltip,
}: {
  value: string;
  defaultValue: string;
  onChange: (color: string) => void;
  tooltip: string;
}) => (
  <div className="flex items-center gap-2">
    <Tooltip content={tooltip}>
      <input
        type="color"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="h-7 w-10 rounded cursor-pointer bg-transparent border border-borderSubtle"
      />
    </Tooltip>
    {value.toLowerCase() !== defaultValue.toLowerCase() && (
      <button
        onClick={() => onChange(defaultValue)}
        className="text-[11px] text-textSecondary hover:text-text"
      >
        Reset
      </button>
    )}
  </div>
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
  const baseColor = storeEntry?.color || '#9147ff';
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

const ChatSettings = ({ hidePlacement = false }: { hidePlacement?: boolean } = {}) => {
  const { settings, updateSettings } = useAppStore();
  // Phone-shell preferences (see mobile/phonePrefs.ts); only read on the phone.
  const mentionHaptic = usePhonePrefs((s) => s.mentionHaptic);
  const setMentionHaptic = usePhonePrefs((s) => s.setMentionHaptic);

  const stored = settings.chat_design;
  const cd = {
    show_dividers: stored?.show_dividers ?? true,
    alternating_backgrounds: stored?.alternating_backgrounds ?? false,
    message_spacing: stored?.message_spacing ?? 2,
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
    user_card_opens_messages: stored?.user_card_opens_messages ?? true,
    seventv_emote_notices: stored?.seventv_emote_notices ?? true,
    link_previews: stored?.link_previews ?? true,
    link_preview_keep_link: stored?.link_preview_keep_link ?? false,
    shorten_links: stored?.shorten_links ?? true,
    link_preview_trusted_domains: stored?.link_preview_trusted_domains ?? [],
    pinned_collapsed_style: stored?.pinned_collapsed_style ?? 'bar',
    pinned_start_collapsed: stored?.pinned_start_collapsed ?? true,
    polls_start_collapsed: stored?.polls_start_collapsed ?? false,
  };

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
    updateSettings({
      ...settings,
      chat_filters: withHiddenUser(settings.chat_filters, name, scope, hidden),
    });
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
      <SettingsSection
        label="Chat Placement"
        description="Where chat sits next to the player, and what it does when the video goes fullscreen."
      >
        <SettingsRow
          title="Where chat sits"
          description="Dock chat to the left, right, or bottom of the player, or hide it to give the video the whole window."
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
        </SettingsRow>
        <SettingsRow
          title="Chat over fullscreen video"
          description="Keep chatting while the stream fills the screen: the chat panel floats over the video as a translucent column. No extra window."
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
        />
        {(settings.fullscreen_chat?.mode ?? 'overlay') === 'overlay' && (
          <>
            <SettingsRow
              title="Hide with the player controls"
              description="The column fades out when the controls do and comes back when you move the mouse. Hovering the chat or typing keeps it up."
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
            <SettingsRow
              title="Overlay opacity"
              description="How much of the video shows through the chat column."
              control={
                <div className="flex items-center gap-2">
                  <input
                    type="range"
                    min={0}
                    max={100}
                    step={5}
                    value={settings.fullscreen_chat?.opacity ?? 55}
                    onChange={(e) =>
                      updateSettings({
                        ...settings,
                        fullscreen_chat: { ...settings.fullscreen_chat, opacity: Number(e.target.value) },
                      })
                    }
                    className="w-32 accent-accent"
                  />
                  <span className="w-10 text-right text-xs tabular-nums text-textSecondary">
                    {settings.fullscreen_chat?.opacity ?? 55}%
                  </span>
                </div>
              }
            />
            <SettingsRow
              title="Overlay width"
              description="Column width in pixels while fullscreen (240 to 640)."
              control={
                <input
                  type="number"
                  min={240}
                  max={640}
                  step={10}
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
            <SettingsRow
              title="Overlay side"
              description="Auto follows the chat placement (a bottom-docked chat floats on the right)."
            >
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
            </SettingsRow>
          </>
        )}
        {(settings.chat_placement === 'left' || settings.chat_placement === 'right') && (
          <SettingsRow
            title="Reveal on hover"
            description="Keep chat tucked against its edge and slide it out when you move toward that side. The player shrinks to make room, the same as dragging the chat open."
            control={
              <Toggle
                enabled={settings.chat_auto_hide ?? false}
                onChange={() =>
                  updateSettings({ ...settings, chat_auto_hide: !(settings.chat_auto_hide ?? false) })
                }
              />
            }
          />
        )}
      </SettingsSection>
      )}

      {/* The phone's counterpart to Chat Placement: how chat shares the screen
          with landscape video, and how it gets your attention. The overlay's
          opacity, width and side are the SAME keys as the desktop overlay, so
          one preference follows you between the two. */}
      {IS_MOBILE && (
        <SettingsSection
          label="Chat on your phone"
          description="How chat behaves when the phone is on its side, and how a mention reaches you."
        >
          <SettingsRow
            title="Chat in landscape"
            description="Turn the phone sideways and tap the chat button on the player. Chat can float over the video, or take a column beside it."
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
          </SettingsRow>
          {(settings.fullscreen_chat?.phone_layout ?? 'overlay') === 'overlay' && (
            <>
              <SettingsRow
                title="How much video shows through"
                description="Lower is clearer video behind the chat; higher is easier reading."
                control={
                  <div className="flex items-center gap-2">
                    <input
                      type="range"
                      min={0}
                      max={100}
                      step={5}
                      value={settings.fullscreen_chat?.opacity ?? 55}
                      onChange={(e) =>
                        updateSettings({
                          ...settings,
                          fullscreen_chat: { ...settings.fullscreen_chat, opacity: Number(e.target.value) },
                        })
                      }
                      className="w-32 accent-accent"
                    />
                    <span className="text-[12px] text-textMuted tabular-nums w-9 text-right">
                      {settings.fullscreen_chat?.opacity ?? 55}%
                    </span>
                  </div>
                }
              />
              <SettingsRow
                title="How wide"
                description="Never more than half the screen, whatever you pick here."
                control={
                  <div className="flex items-center gap-2">
                    <input
                      type="range"
                      min={240}
                      max={480}
                      step={20}
                      value={Math.min(480, settings.fullscreen_chat?.width ?? 340)}
                      onChange={(e) =>
                        updateSettings({
                          ...settings,
                          fullscreen_chat: { ...settings.fullscreen_chat, width: Number(e.target.value) },
                        })
                      }
                      className="w-32 accent-accent"
                    />
                    <span className="text-[12px] text-textMuted tabular-nums w-12 text-right">
                      {Math.min(480, settings.fullscreen_chat?.width ?? 340)}px
                    </span>
                  </div>
                }
              />
              <SettingsRow title="Which side" description="Where the chat column floats.">
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
              </SettingsRow>
            </>
          )}
          <SettingsRow
            title="Buzz when someone mentions you"
            description="A short vibration when a message says your name, on top of the highlight."
            control={<Toggle enabled={mentionHaptic} onChange={() => setMentionHaptic(!mentionHaptic)} />}
          />
        </SettingsSection>
      )}

      <SettingsSection
        label="Channel Points"
        description="Bonus chest pickup on the channel you are watching."
      >
        <SettingsRow
          title="Auto-claim bonus chests"
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
        id="settings-section-youtube-chat"
        label="YouTube Chat"
        description="Which of YouTube's two chat feeds you read. Changes apply the next time you open a YouTube stream."
      >
        <SettingsRow
          title="Which chat to read"
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
      </SettingsSection>

      <SettingsSection
        id="settings-section-chat-events"
        label="Chat Events"
        description="What live channel activity shows while you watch. Turn any of these off to keep chat clean."
      >
        <SettingsRow
          title="Polls"
          description="Show a live poll card at the top of chat when the streamer runs one, with the running vote tally."
          control={
            <Toggle
              enabled={settings.show_polls ?? true}
              onChange={() => updateSettings({ ...settings, show_polls: !(settings.show_polls ?? true) })}
            />
          }
        />
        <SettingsRow
          title="Predictions"
          description="Show a live prediction card at the top of chat, with the outcomes and how points are stacking up."
          control={
            <Toggle
              enabled={settings.show_predictions ?? true}
              onChange={() =>
                updateSettings({ ...settings, show_predictions: !(settings.show_predictions ?? true) })
              }
            />
          }
        />
        {(settings.show_polls ?? true) && (settings.show_predictions ?? true) && (
          <SettingsRow
            title="When both are running"
            description="Pick which card sits on top when a poll and a prediction run at the same time."
            help="Both cards show either way, stacked one above the other."
          >
            <SegmentedSelect<'prediction-first' | 'poll-first'>
              value={settings.chat_overlay_order ?? 'prediction-first'}
              onChange={(order) => updateSettings({ ...settings, chat_overlay_order: order })}
              options={[
                { value: 'prediction-first', label: 'Prediction on top' },
                { value: 'poll-first', label: 'Poll on top' },
              ]}
            />
          </SettingsRow>
        )}
        <SettingsRow
          title="Channel point redemptions"
          description="Shows a chat row when someone redeems a reward that does not post its own message, like a no-input reward."
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
          title="Collapse gift-sub floods"
          description="Shows one 'gifting N subs' row with the recipients attached when someone gifts a batch, instead of a row per gift."
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
          title="Chat replay on clips"
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
      </SettingsSection>

      {/* Desktop only. Writes .log files into a folder the user picks, and there
          is no user-visible folder to point at on Android. */}
      {!IS_MOBILE && (
      <SettingsSection
        label="Chat Logging"
        description="Keep a text copy of chat on your disk, for searching later or feeding another tool."
      >
        <SettingsRow
          title="Save chat logs"
          description="Writes chat to plain text files as you watch: one folder per channel, one file per day."
          help="The files grow with the chat, so a busy channel adds up over weeks. Delete old days from the folder any time."
          control={
            <Toggle
              enabled={loggingEnabled}
              onChange={() => setLogging({ enabled: !loggingEnabled })}
            />
          }
        />
        {loggingEnabled && (
          <>
            <SettingsRow
              title="Log folder"
              description="Where the files are written. Browse to pick your own folder, Reset to go back to the default."
            >
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
                {(logging.folder ?? '') !== '' && (
                  <button
                    type="button"
                    onClick={() => setLogging({ folder: '' })}
                    className="glass-button-secondary flex-shrink-0 px-2 py-1.5 text-[13px] text-textMuted hover:text-textPrimary"
                  >
                    Reset
                  </button>
                )}
                <button
                  type="button"
                  onClick={openLogFolder}
                  className="glass-button-secondary flex-shrink-0 px-3 py-1.5 text-[13px] text-textSecondary hover:text-textPrimary"
                >
                  Open
                </button>
              </div>
            </SettingsRow>
            <SettingsRow
              title="Only log these channels"
              description="Leave empty to log every channel you open."
            >
              <PanelChannelList
                value={logging.channels ?? []}
                onChange={(channels) => setLogging({ channels })}
              />
            </SettingsRow>
            <SettingsRow
              title="Timestamps"
              description="Start each line with the time it was sent."
              control={
                <Toggle
                  enabled={logging.timestamps ?? true}
                  onChange={() => setLogging({ timestamps: !(logging.timestamps ?? true) })}
                />
              }
            />
            <SettingsRow
              title="Events and moderation"
              description="Also log subscriptions, raids, announcements, timeouts, and deleted messages."
              control={
                <Toggle
                  enabled={logging.include_events ?? true}
                  onChange={() => setLogging({ include_events: !(logging.include_events ?? true) })}
                />
              }
            />
          </>
        )}
      </SettingsSection>
      )}

      <SettingsSection
        label="Chat Design"
        description="Spacing, text size, timestamps, and how names and mentions look in the chat feed."
      >
        <SettingsRow
          title="Lines between messages"
          description="Draws a thin line between messages so a fast chat is easier to scan."
          control={
            <Toggle
              enabled={cd.show_dividers ?? true}
              onChange={() => setDesign({ show_dividers: !(cd.show_dividers ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Striped message rows"
          description="Gives every other message a slightly different background, in your theme's colors, so rows are easier to follow."
          control={
            <Toggle
              enabled={cd.alternating_backgrounds ?? false}
              onChange={() => setDesign({ alternating_backgrounds: !(cd.alternating_backgrounds ?? false) })}
            />
          }
        />

        <SettingsRow
          title={`Message spacing: ${cd.message_spacing ?? 2}px`}
          description="Blank space between one message and the next; more room means fewer messages on screen."
        >
          <input
            type="range"
            min="0"
            max="20"
            step="1"
            value={cd.message_spacing ?? 2}
            onChange={(e) => setDesign({ message_spacing: parseInt(e.target.value) })}
            className="w-full accent-accent cursor-pointer"
          />
        </SettingsRow>

        <SettingsRow
          title={`Text size: ${cd.font_size ?? 14}px`}
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
          title="Flash when you are mentioned"
          description="Briefly flashes any message that mentions or replies to you, so you spot it in a fast chat."
          control={
            <Toggle
              enabled={cd.mention_animation ?? true}
              onChange={() => setDesign({ mention_animation: !(cd.mention_animation ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Show timestamps"
          description="Shows the time each message was sent, next to the name."
          control={
            <Toggle
              enabled={cd.show_timestamps ?? false}
              onChange={() => setDesign({ show_timestamps: !(cd.show_timestamps ?? false) })}
            />
          }
        >
          {cd.show_timestamps && (
            <SettingsRow title="Clock" description="12-hour (7:42 PM) or 24-hour (19:42). Formatted once in the backend.">
              <SegmentedSelect<'12h' | '24h'>
                value={cd.timestamp_format ?? '12h'}
                onChange={(timestamp_format) => setDesign({ timestamp_format })}
                options={[
                  { value: '12h', label: '12h' },
                  { value: '24h', label: '24h' },
                ]}
              />
            </SettingsRow>
          )}
          {cd.show_timestamps && (
            <SettingsRow
              title="Include seconds"
              description="Shows seconds too, so 7:42 PM reads 7:42:30 PM."
              control={
                <Toggle
                  enabled={cd.show_timestamp_seconds ?? false}
                  onChange={() => setDesign({ show_timestamp_seconds: !(cd.show_timestamp_seconds ?? false) })}
                />
              }
            />
          )}
        </SettingsRow>

        <SettingsRow
          title="Pins start collapsed"
          description="Shows the pinned message as a compact one-line bar when you enter a channel."
          help="Click the bar to expand it. Turn this off to always open pins fully expanded."
          control={
            <Toggle
              enabled={cd.pinned_start_collapsed ?? true}
              onChange={() => setDesign({ pinned_start_collapsed: !(cd.pinned_start_collapsed ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Polls start collapsed"
          description="Opens live polls as their header bar instead of expanded, so a poll never takes over the top of chat. Tap the header to expand it."
          help="Collapsing a poll sticks: it no longer reopens itself every time somebody votes."
          control={
            <Toggle
              enabled={cd.polls_start_collapsed ?? false}
              onChange={() =>
                setDesign({ polls_start_collapsed: !(cd.polls_start_collapsed ?? false) })
              }
            />
          }
        />

        <SettingsRow
          title="Collapsed pin style"
          description="Shrinks a collapsed pin to a thin one-line bar you can click to expand, or hides it completely."
          help="The bar shows the sender and the start of the message."
        >
          <SegmentedSelect<'bar' | 'hidden'>
            value={cd.pinned_collapsed_style ?? 'bar'}
            onChange={(v) => setDesign({ pinned_collapsed_style: v })}
            options={[
              { value: 'bar', label: 'Bar' },
              { value: 'hidden', label: 'Hidden' },
            ]}
          />
        </SettingsRow>

        <div className="space-y-2">
          <div className="flex items-baseline gap-2">
            <span className="text-xs font-medium text-textSecondary uppercase tracking-wider">Preview</span>
            <span className="text-[11px] text-textMuted">how your name looks in chat</span>
          </div>
          <NamePrefixPreview
            separator={cd.username_separator ?? 'none'}
            nameStyle={cd.username_style ?? 'plain'}
            accentSource={cd.username_accent_source ?? 'user'}
          />
        </div>

        <SettingsRow
          title="Name separator"
          description="The mark between a name and its message, like a colon or an arrow."
          help="Action messages (/me) never get a separator."
        >
          <Dropdown<'none' | 'colon' | 'dot' | 'arrow' | 'pipe' | 'dash'>
            value={cd.username_separator ?? 'none'}
            onChange={(v) => setDesign({ username_separator: v })}
            className="w-full"
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
        </SettingsRow>

        <SettingsRow
          title="Name style"
          description="How names stand out from the message: plain, or with a bar, chip, brackets, or dot."
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
        </SettingsRow>

        {(cd.username_separator !== 'none' || cd.username_style !== 'plain') && (
          <SettingsRow
            title="Prefix color"
            description="Colors the separator, bar, dot, brackets, or chip with the chatter's own color or your theme accent."
          >
            <SegmentedSelect<'user' | 'theme'>
              value={cd.username_accent_source ?? 'user'}
              onChange={(v) => setDesign({ username_accent_source: v })}
              options={[
                { value: 'user', label: 'User color' },
                { value: 'theme', label: 'Theme accent' },
              ]}
            />
          </SettingsRow>
        )}

        <SettingsRow
          title="Mention color"
          description="The highlight color on messages that mention you."
        >
          <ColorSwatch
            value={cd.mention_color ?? '#ff4444'}
            defaultValue="#ff4444"
            onChange={(color) => setDesign({ mention_color: color })}
            tooltip="Mention color"
          />
        </SettingsRow>

        <SettingsRow
          title="Reply thread color"
          description="The color that marks replies in a thread."
        >
          <ColorSwatch
            value={cd.reply_color ?? '#ff6b6b'}
            defaultValue="#ff6b6b"
            onChange={(color) => setDesign({ reply_color: color })}
            tooltip="Reply thread color"
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Link Previews"
        description="Turn links in chat into preview cards, and choose which sites are allowed to expand on their own."
      >
        <SettingsRow
          title="How links show"
          description="Off keeps links as plain text, Card + Link adds a preview card under the link, and Clean shows only the card."
          help="In Clean, hover the card to see where it goes. StreamNook fetches the page from your PC to build the card, so the site sees a visit from you."
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
        </SettingsRow>

        <SettingsRow
          title="Shorten links"
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
          title="Trusted sites"
          description="Links from trusted sites expand into a preview on their own; every other link shows a Load preview button instead."
          help="The shield on a Load preview button trusts that site from chat. Popular sites are trusted out of the box; add or remove your own here."
          disabled={!cd.link_previews}
        >
          <TrustedSourcesEditor
            domains={cd.link_preview_trusted_domains}
            onChange={(next) => setDesign({ link_preview_trusted_domains: next })}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        label="Emotes"
        description="How big emotes are, how they animate, and how much they grow when you hover one."
      >
        <SettingsRow
          title="Animate emotes"
          description="Play animated emotes always, only while you hover a message, or never (first frame). Never is the lightest on the GPU in a fast chat."
        >
          <SegmentedSelect<'always' | 'hover' | 'never'>
            value={cd.animate_emotes ?? 'always'}
            onChange={(animate_emotes) => setDesign({ animate_emotes })}
            options={[
              { value: 'always', label: 'Always' },
              { value: 'hover', label: 'On hover' },
              { value: 'never', label: 'Never' },
            ]}
          />
        </SettingsRow>
        <SettingsRow
          title="Show GIFs in chat"
          description="Twitch lets Tier 2 and Tier 3 subscribers post GIFs. Off swaps each one for a small chip you can click to reveal."
          help="GIFs also follow Animate emotes: Never shows the chip, On hover plays them while you hover the message."
          control={
            <Toggle
              enabled={cd.show_chat_gifs ?? true}
              onChange={() => setDesign({ show_chat_gifs: !(cd.show_chat_gifs ?? true) })}
            />
          }
        />
        <SettingsRow
          title={`History opacity: ${cd.backfill_opacity ?? 100}%`}
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
          title={`Emote size: ${(cd.emote_scale ?? 1).toFixed(2)}x`}
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

        <SettingsRow
          title={`Emote hover size: ${(HOVER_SIZE_OPTIONS.find((o) => o.px === cd.emote_hover_size) ?? HOVER_SIZE_OPTIONS[1]).label}`}
          description={
            cd.compact_emote_tooltips
              ? 'Off while Compact emote tooltips is on, since that replaces the hover card with just the emote name.'
              : 'How large an emote grows when you hover it, in chat and in the emote menu.'
          }
          help="Hover the sample below to try the chosen size. The size of emotes in the message still follows Emote size above."
          disabled={cd.compact_emote_tooltips}
        >
          <div className="space-y-3">
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
        </SettingsRow>

        <SettingsRow
          title={`Emote spacing: ${(cd.emote_margin ?? 0.125).toFixed(3)}rem`}
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
      </SettingsSection>

      <SettingsSection
        label="Chat Input"
        description="Small conveniences in the box where you type, and which buttons sit around it."
      >
        <SettingsRow
          title="Send the same message twice"
          description="Adds an invisible character when you repeat a message, so Twitch does not reject the second send."
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
            title="Ctrl+Enter sends and keeps the text"
            description="Sends the message and leaves it in the box, so you can send it again straight away."
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
          title="Check spelling as you type"
          description="Underlines misspelled words in the message box and offers corrections when you right-click one. Emotes, chatters, commands and links are left alone."
          control={
            <Toggle
              enabled={settings.chat_input?.spellcheck_enabled ?? true}
              onChange={() => setInput({ spellcheck_enabled: !(settings.chat_input?.spellcheck_enabled ?? true) })}
            />
          }
        />
        {(settings.chat_input?.spellcheck_enabled ?? true) && <SpellcheckDictionary />}
        <SettingsRow
          title="Hide the placeholder text"
          description="Leaves the message box empty instead of prompting you to send a message. Notices you can act on, like read-only or subscriber-only mode, still show."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_placeholder ?? false}
              onChange={() => setInput({ hide_placeholder: !(settings.chat_input?.hide_placeholder ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Hide the command button"
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
          title="Hide the emote button"
          description="Removes the smiley from inside the message box. The emote picker is still reachable from its keyboard shortcut and from tab completion."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_emote_button ?? false}
              onChange={() => setInput({ hide_emote_button: !(settings.chat_input?.hide_emote_button ?? false) })}
            />
          }
        />
        <SettingsRow
          title="Hide the points balance"
          description="Removes the channel points button next to the message box. It comes back on its own whenever a bonus chest is waiting, so you never miss one."
          control={
            <Toggle
              enabled={settings.chat_input?.hide_points_balance ?? false}
              onChange={() => setInput({ hide_points_balance: !(settings.chat_input?.hide_points_balance ?? false) })}
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
        label="Emote Tab Completion"
        description={
          IS_MOBILE
            ? 'Type part of an emote name in chat to see matching emotes above the input. Swipe the strip to see more, tap one to use it.'
            : 'Type part of an emote name in chat and press Tab to cycle through matching emotes. Shift+Tab cycles backwards.'
        }
        id="settings-section-emote-tab-completion"
      >
        <SettingsRow
          title="Complete emote names with Tab"
          description={
            IS_MOBILE
              ? 'Suggest matching emotes as you type.'
              : 'Press Tab while typing to insert the best-matching emote, and Tab again to cycle to the next match.'
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
        />
        <SettingsRow
          title="How names match"
          description="Starts With needs the emote to begin with what you typed; Contains matches it anywhere in the name."
        >
          <SegmentedSelect<'starts_with' | 'includes'>
            value={settings.chat_input?.emote_tab_complete_match_mode ?? 'starts_with'}
            options={[
              { value: 'starts_with', label: 'Starts With' },
              { value: 'includes', label: 'Contains' },
            ]}
            onChange={(v) => setInput({ emote_tab_complete_match_mode: v })}
          />
        </SettingsRow>


        <SettingsRow
          title="Complete chatter names too"
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
        label="Render Style"
        description="How deleted messages, mentions, modifier emotes, and a few other special cases are drawn."
      >
        <SettingsRow
          title="Deleted messages"
          description="What happens to a message once it is deleted or its sender is timed out or banned: crossed out, dimmed, left as is, or removed."
        >
          <SegmentedSelect<'strikethrough' | 'dimmed' | 'keep' | 'hidden'>
            value={cd.deleted_message_style as 'strikethrough' | 'dimmed' | 'keep' | 'hidden'}
            onChange={(value) => setDesign({ deleted_message_style: value })}
            options={[
              { value: 'strikethrough', label: 'Strikethrough' },
              { value: 'dimmed', label: 'Dimmed' },
              { value: 'keep', label: 'Keep' },
              { value: 'hidden', label: 'Hidden' },
            ]}
          />
        </SettingsRow>

        <SettingsRow
          title="Hide shared chat messages"
          description="Hides messages that came from the other channel in a Twitch Shared Chat, so you only see this channel's own chatters."
          control={
            <Toggle
              enabled={cd.hide_shared_chat}
              onChange={() => setDesign({ hide_shared_chat: !cd.hide_shared_chat })}
            />
          }
        />

        <SettingsRow
          title="Paint @mentions inline"
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
          title="Compact emote tooltips"
          description='Show just the emote name on hover instead of the full "Right-click to copy" hint.'
          control={
            <Toggle
              enabled={cd.compact_emote_tooltips}
              onChange={() => setDesign({ compact_emote_tooltips: !cd.compact_emote_tooltips })}
            />
          }
        />

        <SettingsRow
          title="FFZ emote effects"
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
          title="BetterTTV emote modifiers"
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
          description={'Draws the last emote of a "Gigantify an Emote" power-up message at 4x below the message, like Twitch does.'}
          help="Off shows the emote inline at its normal size."
          control={
            <Toggle
              enabled={cd.giant_emotes}
              onChange={() => setDesign({ giant_emotes: !cd.giant_emotes })}
            />
          }
        />

        <SettingsRow
          title="7TV emote update notices"
          description="Shows a chat notice when a mod adds, removes, or renames a 7TV emote in the channel."
          help="The new emote is usable right away either way."
          control={
            <Toggle
              enabled={cd.seventv_emote_notices ?? true}
              onChange={() => setDesign({ seventv_emote_notices: !(cd.seventv_emote_notices ?? true) })}
            />
          }
        />

        <SettingsRow
          title="Smooth scroll on Resume"
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
          title={`Message buffer: ${Math.min(IS_MOBILE ? 300 : 1000, settings.chat_render?.message_buffer_cap ?? 100)} messages`}
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
        id="settings-section-repeated-messages"
        label="Repeated Messages"
        description="When several people post the same thing at once, fold the run into one row with a count instead of repeating it down the whole chat."
      >
        <SettingsRow
          title="When a message repeats"
          description={
            repeatMode === 'collapse'
              ? 'Keeps the first one and counts the rest onto it.'
              : repeatMode === 'label'
                ? 'Leaves every message in chat and just numbers them, so nothing is hidden.'
                : 'Repeats are left completely alone.'
          }
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
        </SettingsRow>
        {repeatMode !== 'off' && (
          <>
            <SettingsRow
              title="How closely they must match"
              description='"Nearly the same" ignores capitals, extra spaces and trailing punctuation, so "LULW!!" joins "lulw".'
            >
              <SegmentedSelect<RepeatMatchMode>
                value={rp?.match ?? 'normalized'}
                onChange={(match) => setRepeat({ match })}
                options={[
                  { value: 'normalized', label: 'Nearly the same' },
                  { value: 'exact', label: 'Exactly the same' },
                ]}
              />
            </SettingsRow>
            <SettingsRow
              title={`Show the count from ${repeatThreshold} copies`}
              description="How many copies it takes before the counter appears."
            >
              <input
                type="range"
                min={2}
                max={10}
                step={1}
                value={repeatThreshold}
                onChange={(e) => setRepeat({ threshold: Number(e.target.value) })}
                className="w-full accent-accent cursor-pointer"
              />
            </SettingsRow>
            <SettingsRow
              title={`Group copies sent within ${repeatWindow}s`}
              description="After this long, the next copy starts a fresh run instead of joining the old one."
            >
              <input
                type="range"
                min={10}
                max={300}
                step={5}
                value={repeatWindow}
                onChange={(e) => setRepeat({ window_seconds: Number(e.target.value) })}
                className="w-full accent-accent cursor-pointer"
              />
            </SettingsRow>
            <SettingsRow
              title="Counter colour"
              description="The colour of the little x12 next to the message."
              control={
                <ColorSwatch
                  value={rp?.color || REPEAT_DEFAULT_COLOR}
                  defaultValue={REPEAT_DEFAULT_COLOR}
                  onChange={(color) => setRepeat({ color })}
                  tooltip="Pick the counter colour"
                />
              }
            />
            <SettingsRow
              title="Never fold mods, VIPs or the streamer"
              description="Their messages always stay on their own row, so you can see exactly who said what."
              control={
                <Toggle
                  enabled={rp?.exempt_privileged !== false}
                  onChange={() => setRepeat({ exempt_privileged: rp?.exempt_privileged === false })}
                />
              }
            />
            <SettingsRow
              title="Show everything in channels you moderate"
              description="Turns folding off wherever you're a mod, so a hidden copy can never be a message you needed to action."
              control={
                <Toggle
                  enabled={rp?.keep_all_when_moderator !== false}
                  onChange={() => setRepeat({ keep_all_when_moderator: rp?.keep_all_when_moderator === false })}
                />
              }
            />
          </>
        )}
      </SettingsSection>

      <SettingsSection
        id="settings-section-chat-filters"
        label="Hidden Users & Bots"
        description="Stop chosen users' messages from reaching your chat, in one channel or everywhere. Only affects what you see; nothing is sent to the platform, and your own messages are never hidden."
      >
        <SettingsRow
          title="Hide known bots"
          description="StreamElements, Nightbot, Moobot and the other well-known chat bots, in every channel."
        >
          <Toggle
            enabled={cfs?.hide_bots ?? false}
            onChange={() => setChatFilters({ hide_bots: !(cfs?.hide_bots ?? false) })}
          />
        </SettingsRow>
        <SettingsRow
          title="Hidden everywhere"
          description="Messages from these names never appear, on any platform. Add someone here, or from their user card in chat."
        >
          <HiddenNameEditor
            names={cfs?.hidden_users ?? []}
            onAdd={(n) => setHidden(n, 'global', true)}
            onRemove={(n) => setHidden(n, 'global', false)}
          />
        </SettingsRow>
        <SettingsRow
          title="Ignored phrases"
          description="Never see messages that contain these words, in any chat. The sender is not told. Plain words work; turn on regex or whole-word per phrase when you need precision."
        >
          <IgnoredPhrasesSettings />
        </SettingsRow>
        {perChannelHidden.length > 0 && (
          <SettingsRow
            title="Hidden in one channel"
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
      </SettingsSection>

      {/* Desktop only: filters are applied from the funnel in a chat's header
          and search opens with Ctrl+F, and the phone chat pane has neither. */}
      {!IS_MOBILE && (
      <SettingsSection
        id="settings-section-chat-query"
        label="Message Filters & Search"
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
          title="How far back search reaches"
          description="Messages remembered per chat for Ctrl+F. Higher finds older messages; 1000 is roughly a megabyte per open chat."
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
          title="Open on their messages"
          description="Land on the person's recent chat history straight away. Off opens the profile first, with their badges and stats. Either way the card switches between the two."
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

      <SettingsSection
        label="7TV Cosmetics"
        description="How 7TV paints on usernames are drawn."
      >
        <SettingsRow
          title="Paint drop shadows"
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
