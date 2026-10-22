import React, { useRef, useEffect, useMemo, useState, useCallback } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { useChatConnectionStore } from '../../stores/chatConnectionStore';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { motion, AnimatePresence } from 'framer-motion';
import type { ModLogEvent } from '../../types';
import { openProfilePopup } from '../../utils/openProfilePopup';
import { colorForAction, highlightContainerStyle, type HighlightStyleKey } from '../../utils/modLogCategories';
import { useTwitchIdentity } from '../../utils/avatarCache';
import { SevenTvAvatar } from '../SevenTvAvatar';
import { sevenTvIdForStream } from '../../services/sevenTvPictures';
import { Tooltip } from '../ui/Tooltip';
import { usePluginUiRegistry, selectSlot } from '../../plugins-ui/registry';
import { invoke } from '@tauri-apps/api/core';
import { parseKey } from '../../utils/providerKey';
import { ProviderLogo } from '../ProviderLogo';
import type { ProviderId } from '../../types/providers';
import { moderateWithToast } from '../../services/moderation';
import { isCachedModerator } from '../../services/modRoomService';
import { badgesModerate, liftActions, nextLiftExpiry, type LiftKind } from './modLogLift';
import { GLAZE_PILL } from '../ui/glazeButtons';

/** An entry's Unban or Lift timeout, when it offers one. */
interface RowLift {
  kind: LiftKind;
  busy: boolean;
  run: () => void;
}

// Newest entries are prepended at the top. Keep the view pinned to the top (so
// the newest is always visible and older entries slide down beneath it) while
// the user is within this many pixels of the top; if they've scrolled further
// down to read history, leave their position alone.
const TOP_STICK_PX = 60;

function humanDuration(secs?: number): string | undefined {
  if (secs === undefined || secs === null || Number.isNaN(secs)) return undefined;
  if (secs >= 86400) return `${Math.round(secs / 86400)}d`;
  if (secs >= 3600) return `${Math.round(secs / 3600)}h`;
  if (secs >= 60) return `${Math.round(secs / 60)}m`;
  return `${secs}s`;
}

// A small round avatar resolved by login, with an initial fallback while it loads
// (or if the user has none). The login lookup also yields the Twitch id, which
// draws the person's 7TV picture.
const Avatar: React.FC<{ login?: string; name?: string; size?: number; url?: string; sevenTvId?: string | null }> = ({
  login,
  name,
  size = 18,
  url,
  sevenTvId,
}) => {
  // `url` is an explicit override (e.g. a Kick channel's profile_pic, which can't
  // resolve through the Twitch-login-keyed avatar cache). Falls back to it.
  const fetched = useTwitchIdentity(login);
  const src = url || fetched?.url;
  const cosmeticsId = sevenTvId ?? fetched?.id ?? null;
  const initial = (name || login || '?').charAt(0).toUpperCase();
  return (
    <span
      className="inline-flex items-center justify-center rounded-full overflow-hidden bg-background text-textSecondary font-medium flex-shrink-0 select-none"
      style={{ width: size, height: size, fontSize: Math.round(size * 0.5) }}
    >
      {src ? (
        <SevenTvAvatar
          cosmeticsId={cosmeticsId}
          px={size}
          src={src}
          alt=""
          className="w-full h-full object-cover"
          referrerPolicy="no-referrer"
        />
      ) : (
        initial
      )}
    </span>
  );
};

// Short, scannable label for the header (the colored severity word). The
// participants line carries who/whom; this is just the action.
function labelForAction(log: ModLogEvent): { label: string; detail?: string } {
  const a = (log.action || '').toLowerCase();
  const msgs = (n: number) => `${n} msg${n === 1 ? '' : 's'}`;
  switch (a) {
    case 'ban':
      return { label: 'Banned', detail: log.removed_count ? msgs(log.removed_count) : undefined };
    case 'timeout': return { label: 'Timed out', detail: humanDuration(log.duration) };
    // YouTube author-removal: a non-mod reader can't tell a timeout from a permanent
    // ban (both arrive as the same "remove all by author" action with no duration), so
    // surface it honestly as a removal with the message count rather than guessing.
    case 'removed':
      return { label: 'Removed', detail: log.removed_count ? msgs(log.removed_count) : undefined };
    case 'delete': return { label: 'Message deleted' };
    case 'clear':
    case 'clear_chat': return { label: 'Chat cleared' };
    case 'unban': return { label: 'Unbanned' };
    case 'untimeout': return { label: 'Timeout lifted' };
    case 'warn': return { label: 'Warned' };
    case 'vip': return { label: 'VIP granted' };
    case 'unvip': return { label: 'VIP removed' };
    case 'mod': return { label: 'Modded' };
    case 'unmod': return { label: 'Unmodded' };
    case 'raid': return { label: 'Raid started' };
    case 'unraid': return { label: 'Raid cancelled' };
    case 'emoteonly':
    case 'emote_only_on': return { label: 'Emote-only on' };
    case 'emoteonlyoff':
    case 'emote_only_off': return { label: 'Emote-only off' };
    case 'followers':
    case 'follower_only_on': return { label: 'Followers-only on' };
    case 'followersoff':
    case 'follower_only_off': return { label: 'Followers-only off' };
    case 'subscribers':
    case 'subscriber_only_on': return { label: 'Subs-only on' };
    case 'subscribersoff':
    case 'subscriber_only_off': return { label: 'Subs-only off' };
    case 'slow':
    case 'slow_mode_on': return { label: 'Slow mode on' };
    case 'slowoff':
    case 'slow_mode_off': return { label: 'Slow mode off' };
    case 'uniquechat': return { label: 'Unique-chat on' };
    case 'uniquechatoff': return { label: 'Unique-chat off' };
    case 'add_blocked_term': return { label: 'Blocked term added' };
    case 'remove_blocked_term': return { label: 'Blocked term removed' };
    case 'add_permitted_term': return { label: 'Permitted term added' };
    case 'remove_permitted_term': return { label: 'Permitted term removed' };
    case 'approve_unban_request': return { label: 'Unban approved' };
    case 'deny_unban_request': return { label: 'Unban denied' };
    default:
      return { label: (a.replace(/_/g, ' ') || 'action').replace(/^\w/, (c) => c.toUpperCase()) };
  }
}

const ModLogRow: React.FC<{
  log: ModLogEvent;
  colors?: Record<string, string>;
  highlightStyle: HighlightStyleKey;
  showChannel: boolean;
  /** Resolved source identity for the per-entry chip in the combined view: provider
   *  (for the logo), normalized display name, and avatar url (non-Twitch). */
  channelMeta?: { provider: ProviderId; name: string; avatarUrl?: string };
  lift?: RowLift;
}> = ({ log, colors, highlightStyle, showChannel, channelMeta, lift }) => {
  const color = colorForAction(log.action, colors);
  const { label, detail } = labelForAction(log);

  const open = (e: React.MouseEvent, id?: string, login?: string, name?: string) => {
    if (!login) return; // id is optional — openProfilePopup resolves it from the login
    void openProfilePopup({
      userId: id || '',
      username: login.toLowerCase(),
      displayName: name || login,
      // The login (not the display name): badge + IVR lookups and the
      // channel-id resolution in openProfilePopup all key off the channel
      // login. Without the right channel, badges resolve in the viewed user's
      // own channel and everyone shows a broadcaster badge.
      channelName: log.channel || log.channel_display || '',
      clientX: e.clientX,
      clientY: e.clientY,
    });
  };

  const modName =
    log.moderator_name &&
    log.moderator_name !== 'Unknown' &&
    log.moderator_name !== 'A moderator' &&
    log.moderator_name !== 'Twitch System'
      ? log.moderator_name
      : undefined;
  const targetName =
    log.target_user_name && log.target_user_name !== 'Stream/Settings'
      ? log.target_user_name
      : undefined;
  const isSelf = !!(modName && targetName && modName.toLowerCase() === targetName.toLowerCase());
  const showArrow = !!modName && !!targetName && !isSelf;
  const modClickable = !!log.moderator_login;
  const targetClickable = !!log.target_user_login;

  const time = new Date(log.timestamp).toLocaleTimeString([], {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
  const channelLabel = log.channel_display || log.channel;
  const hasParticipants = !!modName || (!!targetName && !isSelf) || isSelf;
  const showChannelChip = showChannel && !!channelLabel;
  const nameClass = 'hover:underline cursor-pointer';
  const labelStyle: React.CSSProperties = highlightStyle === 'box' ? {} : { color };

  return (
    <motion.div
      layout
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ layout: { type: 'spring', stiffness: 500, damping: 40 }, opacity: { duration: 0.2 } }}
      style={highlightContainerStyle(highlightStyle, color)}
      className="group rounded-md px-2.5 py-2 bg-secondary/60 border border-borderSubtle"
    >
      {/* Header: severity dot + action label, time on the right; an undo
          for a ban or running timeout shows beside the time on hover or
          focus, its room always kept so nothing shifts. */}
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="h-2 w-2 rounded-full flex-shrink-0" style={{ backgroundColor: color }} />
          <span
            className={`text-[13px] font-semibold truncate ${highlightStyle === 'box' ? 'text-text' : ''}`}
            style={labelStyle}
          >
            {label}
            {detail && <span className="font-normal opacity-80"> · {detail}</span>}
          </span>
        </div>
        <div className="flex flex-shrink-0 items-center gap-2.5">
          {lift && (
            <button
              type="button"
              onClick={lift.run}
              disabled={lift.busy}
              aria-label={`${lift.kind === 'unban' ? 'Unban' : 'Lift the timeout on'} ${targetName ?? 'this chatter'}`}
              className={`${GLAZE_PILL} -my-0.5 h-6 px-2.5 text-xs !transition-[opacity,background-color,filter] focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent ${
                lift.busy ? 'opacity-60' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100'
              }`}
            >
              {lift.kind === 'unban' ? 'Unban' : 'Lift timeout'}
            </button>
          )}
          <time className="text-[11px] text-textSecondary tabular-nums">{time}</time>
        </div>
      </div>

      {/* Who did it / to whom — moderator pfp + name, then target */}
      {hasParticipants && (
        <div className="mt-1.5 ml-4 flex flex-wrap items-center gap-1.5 text-[12px] text-textSecondary min-w-0">
          {modName && (
            <span className="flex items-center gap-1.5 min-w-0">
              {log.moderator_login && (
                <Avatar login={log.moderator_login} name={modName} size={16} sevenTvId={log.moderator_id} />
              )}
              {modClickable ? (
                <Tooltip content={`View ${modName}'s profile`}>
                  <span
                    className={`text-text ${nameClass}`}
                    onClick={(e) => open(e, log.moderator_id, log.moderator_login, modName)}
                  >
                    {modName}
                  </span>
                </Tooltip>
              ) : (
                <span className="text-text">{modName}</span>
              )}
            </span>
          )}
          {showArrow && <span className="text-textSecondary/60">&rarr;</span>}
          {targetName &&
            !isSelf &&
            (targetClickable ? (
              <Tooltip content={`View ${targetName}'s profile`}>
                <span
                  className={`text-text ${nameClass}`}
                  onClick={(e) => open(e, log.target_user_id, log.target_user_login, targetName)}
                >
                  {targetName}
                </span>
              </Tooltip>
            ) : (
              <span className="text-text">{targetName}</span>
            ))}
        </div>
      )}

      {/* The message involved in the action (e.g. the deleted message text) */}
      {log.message && (
        <div className="mt-1.5 ml-4 text-[12px] text-text break-words rounded bg-background/60 border-l-2 border-borderSubtle px-2 py-1">
          {log.message}
        </div>
      )}

      {/* Channel (only when combining multiple channels) + reason */}
      {(showChannelChip || log.reason) && (
        <div className="mt-1.5 ml-4 flex flex-wrap items-center gap-x-2 gap-y-1 min-w-0">
          {showChannelChip && (
            <span className="flex items-center gap-1.5 min-w-0">
              {channelMeta && (
                <ProviderLogo provider={channelMeta.provider} size={12} className="flex-shrink-0" />
              )}
              <Avatar
                login={!channelMeta || channelMeta.provider === 'twitch' ? log.channel : undefined}
                name={channelMeta?.name ?? channelLabel}
                size={14}
                url={channelMeta && channelMeta.provider !== 'twitch' ? channelMeta.avatarUrl : undefined}
                sevenTvId={
                  channelMeta?.provider === 'youtube'
                    ? sevenTvIdForStream({ provider: 'youtube', user_id: log.channel })
                    : undefined
                }
              />
              <span className="text-[11px] text-textSecondary truncate max-w-[140px]">
                {channelMeta?.name ?? channelLabel}
              </span>
            </span>
          )}
          {log.reason && (
            <span className="text-[11px] italic text-textSecondary truncate">"{log.reason}"</span>
          )}
        </div>
      )}
    </motion.div>
  );
};

// One channel's column in the split view. Owns its own scroll so each channel's
// log scrolls independently, and auto-sticks to the newest entry.
const ModLogColumn: React.FC<{
  channel: { login: string; name: string; provider: ProviderId; channel: string };
  avatarUrl?: string;
  logs: ModLogEvent[];
  colors?: Record<string, string>;
  highlightStyle: HighlightStyleKey;
  isFirst: boolean;
  liftFor: (log: ModLogEvent) => RowLift | undefined;
}> = ({ channel, avatarUrl, logs, colors, highlightStyle, isFirst, liftFor }) => {
  const scrollRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = scrollRef.current;
    if (el && el.scrollTop <= TOP_STICK_PX) el.scrollTop = 0;
  }, [logs.length]);

  return (
    <div
      className={`flex min-w-0 flex-1 flex-col overflow-hidden ${
        isFirst ? '' : 'border-l border-borderSubtle'
      }`}
    >
      <div className="flex flex-shrink-0 items-center gap-1.5 border-b border-borderSubtle bg-secondary/30 px-2 py-1">
        <Avatar
          login={channel.provider === 'twitch' ? channel.channel : undefined}
          name={channel.name}
          size={14}
          url={channel.provider !== 'twitch' ? avatarUrl : undefined}
          sevenTvId={channel.provider === 'youtube' ? sevenTvIdForStream({ provider: 'youtube', user_id: channel.channel }) : undefined}
        />
        <ProviderLogo provider={channel.provider} size={10} className="flex-shrink-0" />
        <span className="truncate text-[12px] font-medium text-textSecondary">{channel.name}</span>
        {logs.length > 0 && (
          <span className="ml-auto flex-shrink-0 rounded-full bg-background px-1.5 text-[10px] text-textMuted">
            {logs.length}
          </span>
        )}
      </div>
      <div
        ref={scrollRef}
        style={{ overflowAnchor: 'none' }}
        className="flex-1 space-y-1.5 overflow-y-auto scrollbar-thin scrollbar-thumb-accent scrollbar-track-transparent p-1.5"
      >
        {logs.length === 0 ? (
          <div className="flex h-full items-center justify-center text-xs text-textMuted">No events</div>
        ) : (
          <AnimatePresence initial={false}>
            {logs.map((log) => (
              <ModLogRow
                key={log.id}
                log={log}
                colors={colors}
                highlightStyle={highlightStyle}
                showChannel={false}
                lift={liftFor(log)}
              />
            ))}
          </AnimatePresence>
        )}
      </div>
    </div>
  );
};

// Persisted preference for splitting the logs into one column per channel.
// Default on, so a multi-channel session auto-splits (and collapses to a single
// combined list when only one channel is open). It's a viewing preference, not
// per-window, so we keep it in localStorage shared across windows.
const SPLIT_PREF_KEY = 'streamnook.modlogs.split';
function loadSplitPref(): boolean {
  try {
    return localStorage.getItem(SPLIT_PREF_KEY) !== '0';
  } catch {
    return true;
  }
}

// Persisted per-contribution preference for showing a plugin-docked column
// inside this pane (the `modlogs.dock` slot). Default off; like the split
// pref it's a viewing preference shared across windows.
const dockPrefKey = (id: string) => `streamnook.modlogs.dock.${id}`;
function loadDockPref(id: string): boolean {
  try {
    return localStorage.getItem(dockPrefKey(id)) === '1';
  } catch {
    return false;
  }
}

// `forceShow` lets a host render the pane regardless of the global
// `settings.show_mod_logs` preference. The main app leaves it unset (the pane
// follows the global setting); the MultiChat popout passes `forceShow` because
// it owns its own per-window mod-logs toggle and shouldn't be coupled to the
// main window's setting.
export const ModLogsWidget: React.FC<{
  forceShow?: boolean;
  /** login -> display name, supplied by hosts that know proper capitalization
   *  (e.g. the MultiChat popout's tab list). Falls back to MultiNook slots /
   *  the watched stream / the bare login when absent. */
  channelLabels?: Record<string, string>;
  /** When provided, renders a gear in the header that jumps straight to the
   *  moderation settings. Only the main window passes this — the MultiChat
   *  popout doesn't host the full settings overlay, so it leaves it off rather
   *  than show a button that goes nowhere. */
  onOpenSettings?: () => void;
}> = ({ forceShow, channelLabels, onOpenSettings }) => {
  // Actions without a subscription; state through individual selectors. This
  // was a whole-store subscription on a component that is also woken by every
  // chat revision bump below, so it re-rendered on essentially everything.
  const { clearModLogs, loadModLogsForChannel, pruneModLogsToChannels } = useAppStore.getState();
  const modLogs = useAppStore((s) => s.modLogs);
  const settings = useAppStore((s) => s.settings);
  const currentStream = useAppStore((s) => s.currentStream);
  const channelsInPopouts = useAppStore((s) => s.channelsInPopouts);
  const slots = usemultiNookStore((s) => s.slots);
  const channelKeys = useChatConnectionStore((s) => Array.from(s.channels.keys()).sort().join(','));
  const logsContainerRef = useRef<HTMLDivElement>(null);
  const modColors = settings.moderation?.mod_log_colors;
  const highlightStyle: HighlightStyleKey = settings.moderation?.mod_log_highlight_style ?? 'box';

  // Channels with a chat open *somewhere* — main-window chats (single stream +
  // MultiNook tiles) plus MultiChat popout chats. Only these contribute to the
  // pane; once you leave a channel (exit the stream, remove a tile, close a
  // popout chat) its entries drop out of the live view. They stay on disk for
  // when you reopen the channel.
  const popoutKeys = useMemo(
    () => Array.from(channelsInPopouts).map((c) => c.toLowerCase()).sort().join(','),
    [channelsInPopouts],
  );
  const activeSet = useMemo(
    () => new Set([...channelKeys.split(','), ...popoutKeys.split(',')].filter(Boolean)),
    [channelKeys, popoutKeys],
  );
  const visibleLogs = useMemo(
    () => modLogs.filter((l) => l.channel && activeSet.has(l.channel.toLowerCase())),
    [modLogs, activeSet],
  );

  // Twitch channels the viewer moderates, as `key=broadcasterId`: their own
  // badges there once USERSTATE lands, the remembered answer until then. Kick
  // and YouTube keep their moderator answer inside the chat pane, so their
  // entries stay read-only here.
  const moderatedKey = useChatConnectionStore((s) => {
    const out: string[] = [];
    for (const [key, slice] of s.channels) {
      if (slice.provider !== 'twitch' || !slice.channelId) continue;
      const badges = slice.userBadges ?? slice.userBadgesFromIrc;
      const mod = badges == null ? isCachedModerator(slice.channelId) : badgesModerate(badges);
      if (mod) out.push(`${key.toLowerCase()}=${slice.channelId}`);
    }
    return out.sort().join(',');
  });
  const moderated = useMemo(() => {
    const m = new Map<string, string>();
    for (const part of moderatedKey.split(',')) {
      const at = part.indexOf('=');
      if (at > 0) m.set(part.slice(0, at), part.slice(at + 1));
    }
    return m;
  }, [moderatedKey]);

  // Unban / Lift timeout on the entries that can still be undone. A lift
  // that went through hides its button at once, since an IRC-only feed never
  // logs the unban that would.
  const [liftNow, setLiftNow] = useState(() => Date.now());
  const [liftBusy, setLiftBusy] = useState<ReadonlySet<string>>(() => new Set());
  const [liftDone, setLiftDone] = useState<ReadonlySet<string>>(() => new Set());
  const lifts = useMemo(
    () => liftActions(visibleLogs, liftNow, (ch) => moderated.has(ch)),
    [visibleLogs, liftNow, moderated],
  );
  // A Lift timeout goes away when its timeout runs out.
  useEffect(() => {
    const at = nextLiftExpiry(visibleLogs, lifts);
    if (at == null) return;
    const t = window.setTimeout(() => setLiftNow(Date.now()), Math.max(0, at - Date.now()) + 50);
    return () => window.clearTimeout(t);
  }, [visibleLogs, lifts]);
  const runLift = async (log: ModLogEvent, kind: LiftKind) => {
    const channel = (log.channel || '').toLowerCase();
    const broadcasterId = moderated.get(channel);
    if (!broadcasterId || !log.target_user_id) return;
    setLiftBusy((s) => new Set(s).add(log.id));
    const outcome = await moderateWithToast({
      provider: 'twitch',
      broadcasterId,
      channel,
      targetUserId: log.target_user_id,
      targetName: log.target_user_name || log.target_user_login || null,
      action: { kind: 'unban', timeout: kind === 'untimeout' },
    });
    setLiftBusy((s) => {
      const next = new Set(s);
      next.delete(log.id);
      return next;
    });
    if (outcome) setLiftDone((s) => new Set(s).add(log.id));
  };
  const liftFor = (log: ModLogEvent): RowLift | undefined => {
    const kind = lifts.get(log.id);
    if (!kind || liftDone.has(log.id)) return undefined;
    return { kind, busy: liftBusy.has(log.id), run: () => void runLift(log, kind) };
  };

  // With more than one channel visible (combined modding), each entry shows its
  // own channel; with one, the header carries it so entries stay clean.
  const distinctChannels = useMemo(
    () => Array.from(new Set(visibleLogs.map((l) => (l.channel || '').toLowerCase()).filter(Boolean))),
    [visibleLogs],
  );
  const showChannelPerEntry = distinctChannels.length > 1;

  // Every channel with an open chat in this window — the accounts you're
  // actively watching. Drives the header chips so your whole lineup shows at a
  // glance, even channels that haven't produced a mod event yet. (Previously the
  // header derived a single channel from the entries that existed, so a quiet
  // second chat never appeared in it.)
  const activeChannels = useMemo(() => {
    const labelFor = (login: string): string => {
      if (channelLabels && channelLabels[login]) return channelLabels[login];
      const slot = slots.find((s) => s.channelLogin.toLowerCase() === login);
      if (slot?.channelName) return slot.channelName;
      if (currentStream?.user_login?.toLowerCase() === login) {
        return currentStream.user_name || login;
      }
      return login;
    };
    return Array.from(activeSet)
      .sort()
      .map((login) => {
        const { provider, channel } = parseKey(login);
        return { login, name: labelFor(login), provider, channel };
      });
  }, [activeSet, channelLabels, slots, currentStream]);

  // Kick + YouTube channels can't resolve through the Twitch avatar cache, so fetch
  // their captured profile_pic (cached backend-side at channel resolve) for the
  // chips. Keyed by `provider:channel` so the same name on two providers can't clash.
  const [providerAvatars, setProviderAvatars] = useState<Record<string, string>>({});
  useEffect(() => {
    const metaChannels = activeChannels.filter((c) => c.provider === 'kick' || c.provider === 'youtube');
    if (metaChannels.length === 0) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let attempts = 0;
    // On a fresh launch the backend hasn't re-resolved the channel yet, so the
    // pic isn't ready on the first pass. Poll until every channel has one (capped,
    // so a channel with no pic doesn't poll forever).
    const poll = async () => {
      if (cancelled) return;
      let missing = false;
      for (const c of metaChannels) {
        try {
          const cmd = c.provider === 'youtube' ? 'get_youtube_channel_meta' : 'get_kick_channel_meta';
          const meta = await invoke<{ profile_pic?: string | null } | null>(cmd, { slug: c.channel });
          if (cancelled) return;
          const mapKey = `${c.provider}:${c.channel}`;
          if (meta?.profile_pic) {
            const url = meta.profile_pic;
            setProviderAvatars((prev) => (prev[mapKey] === url ? prev : { ...prev, [mapKey]: url }));
          } else {
            missing = true;
          }
        } catch {
          missing = true;
        }
      }
      if (!cancelled && missing && ++attempts < 40) timer = setTimeout(poll, 1000);
    };
    void poll();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [activeChannels]);

  // Source identity per channel key, for the combined view's per-entry chip
  // (provider logo + normalized name + avatar). Keyed by the lowercased composite
  // key, matching a log's `channel`.
  const channelMeta = useMemo(() => {
    const m: Record<string, { provider: ProviderId; name: string; avatarUrl?: string }> = {};
    for (const ch of activeChannels) {
      m[ch.login] = {
        provider: ch.provider,
        name: ch.name,
        avatarUrl: ch.provider !== 'twitch' ? providerAvatars[`${ch.provider}:${ch.channel}`] : undefined,
      };
    }
    return m;
  }, [activeChannels, providerAvatars]);

  // Split the logs into a column per channel. Auto-engages with >1 channel;
  // the toggle (header) lets you force the combined list back. With one channel
  // there's nothing to split, so it always shows combined.
  const [splitEnabled, setSplitEnabled] = useState<boolean>(loadSplitPref);
  const canSplit = activeChannels.length > 1;
  const isSplit = splitEnabled && canSplit;
  const toggleSplit = useCallback(() => {
    setSplitEnabled((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(SPLIT_PREF_KEY, next ? '1' : '0');
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  // Plugin-docked columns (the `modlogs.dock` slot): each contribution gets a
  // header toggle and, when on, an even flex slot exactly like a channel
  // column. With a single channel the pane splits logs | docked column.
  // Persisted values are read per contribution set; in-session toggles layer
  // on top (no effect needed, so no setState-in-effect cascade).
  const dockContributions = usePluginUiRegistry(selectSlot('modlogs.dock'));
  const persistedDockPrefs = useMemo(() => {
    const map: Record<string, boolean> = {};
    for (const c of dockContributions) map[c.id] = loadDockPref(c.id);
    return map;
  }, [dockContributions]);
  const [dockOverrides, setDockOverrides] = useState<Record<string, boolean>>({});
  const isDockOn = (id: string) => dockOverrides[id] ?? persistedDockPrefs[id] ?? false;
  const toggleDock = (id: string) => {
    const next = !isDockOn(id);
    try {
      localStorage.setItem(dockPrefKey(id), next ? '1' : '0');
    } catch {
      // ignore
    }
    setDockOverrides((prev) => ({ ...prev, [id]: next }));
  };

  // Load each active channel's persisted history (so reopening restores it), and
  // prune entries + load-guards for channels no longer open anywhere.
  useEffect(() => {
    const active = Array.from(activeSet);
    for (const key of active) void loadModLogsForChannel(key);
    pruneModLogsToChannels(active);
  }, [activeSet, loadModLogsForChannel, pruneModLogsToChannels]);

  // Keep pinned to the top (newest) when the user is at/near the top, so new
  // entries appear at the top and push older ones down. If they've scrolled into
  // history, don't yank them back.
  useEffect(() => {
    const el = logsContainerRef.current;
    if (el && el.scrollTop <= TOP_STICK_PX) el.scrollTop = 0;
  }, [visibleLogs]);

  if (!forceShow && !settings.show_mod_logs) return null;

  return (
    <div className="flex flex-col h-full bg-background border-borderSubtle overflow-hidden">
      {/* Header */}
      <div className="flex-shrink-0 flex items-center justify-between gap-2 px-4 py-2 bg-secondary border-b border-borderSubtle">
        <div className="flex items-center gap-2 min-w-0">
          <svg className="w-4 h-4 text-accent flex-shrink-0" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z" />
          </svg>
          <span className="text-sm font-medium text-text flex-shrink-0">Moderator Logs</span>
          {activeChannels.length > 0 && (
            <span className="flex items-center gap-2 min-w-0 overflow-hidden pl-2 border-l border-borderSubtle">
              {activeChannels.slice(0, 4).map((ch) => (
                <Tooltip key={ch.login} content={ch.name}>
                  <span className="flex items-center gap-1.5 min-w-0">
                    <Avatar
                      login={ch.provider === 'twitch' ? ch.channel : undefined}
                      name={ch.name}
                      size={16}
                      url={ch.provider !== 'twitch' ? providerAvatars[`${ch.provider}:${ch.channel}`] : undefined}
                      sevenTvId={ch.provider === 'youtube' ? sevenTvIdForStream({ provider: 'youtube', user_id: ch.channel }) : undefined}
                    />
                    <ProviderLogo provider={ch.provider} size={11} className="flex-shrink-0" />
                    <span className="text-[13px] text-textSecondary truncate max-w-[110px]">{ch.name}</span>
                  </span>
                </Tooltip>
              ))}
              {activeChannels.length > 4 && (
                <span className="flex-shrink-0 text-[12px] text-textMuted">
                  +{activeChannels.length - 4}
                </span>
              )}
            </span>
          )}
        </div>
        <div className="flex items-center gap-2 flex-shrink-0">
          {canSplit && (
            <Tooltip content={isSplit ? 'Combine into one list' : 'Split into a column per channel'}>
              <button
                onClick={toggleSplit}
                aria-pressed={isSplit}
                className={`p-1 rounded transition-colors ${
                  isSplit ? 'text-accent' : 'text-textSecondary hover:text-text hover:bg-background'
                }`}
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
                  <rect x="3" y="4" width="18" height="16" rx="1.5" />
                  <line x1="9" y1="4" x2="9" y2="20" />
                  <line x1="15" y1="4" x2="15" y2="20" />
                </svg>
              </button>
            </Tooltip>
          )}
          {dockContributions.map((c) => (
            <Tooltip
              key={`${c.pluginId}:${c.id}`}
              content={isDockOn(c.id) ? `Hide ${c.label}` : `Show ${c.label} in this pane`}
            >
              <button
                onClick={() => toggleDock(c.id)}
                aria-pressed={isDockOn(c.id)}
                className={`p-1 rounded transition-colors ${
                  isDockOn(c.id) ? 'text-accent' : 'text-textSecondary hover:text-text hover:bg-background'
                }`}
              >
                <c.Icon className="w-4 h-4" />
              </button>
            </Tooltip>
          ))}
          {visibleLogs.length > 0 && (
            <span className="text-xs text-textSecondary bg-background px-2 py-0.5 rounded-full">
              {visibleLogs.length}
            </span>
          )}
          {onOpenSettings && (
            <Tooltip content="Moderation settings">
              <button
                onClick={onOpenSettings}
                className="text-textSecondary hover:text-text transition-colors p-1 rounded hover:bg-background"
              >
                <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                </svg>
              </button>
            </Tooltip>
          )}
          <Tooltip content="Clear logs (also clears saved history for these channels)">
            <button
              onClick={clearModLogs}
              className="text-textSecondary hover:text-text transition-colors p-1 rounded hover:bg-background"
            >
              <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
              </svg>
            </button>
          </Tooltip>
        </div>
      </div>

      {/* Logs Container — combined single list, or a column per channel when
          split, plus any toggled-on plugin-docked columns. Each takes an even
          flex slot exactly like a channel column. */}
      <div className="flex min-h-0 flex-1 flex-row overflow-hidden">
        {isSplit ? (
          activeChannels.map((ch, i) => (
            <ModLogColumn
              key={ch.login}
              channel={ch}
              avatarUrl={ch.provider !== 'twitch' ? providerAvatars[`${ch.provider}:${ch.channel}`] : undefined}
              logs={visibleLogs.filter((l) => (l.channel || '').toLowerCase() === ch.login)}
              colors={modColors}
              highlightStyle={highlightStyle}
              isFirst={i === 0}
              liftFor={liftFor}
            />
          ))
        ) : (
          <div
            ref={logsContainerRef}
            style={{ overflowAnchor: 'none' }}
            className="min-w-0 flex-1 overflow-y-auto scrollbar-thin scrollbar-thumb-accent scrollbar-track-transparent p-2 space-y-1.5"
          >
            <AnimatePresence initial={false}>
              {visibleLogs.length === 0 ? (
                <motion.div
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  className="flex items-center justify-center h-full text-textSecondary text-sm"
                >
                  No moderation events yet
                </motion.div>
              ) : (
                visibleLogs.map((log) => (
                  <ModLogRow
                    key={log.id}
                    log={log}
                    colors={modColors}
                    highlightStyle={highlightStyle}
                    showChannel={showChannelPerEntry}
                    channelMeta={channelMeta[(log.channel || '').toLowerCase()]}
                    lift={liftFor(log)}
                  />
                ))
              )}
            </AnimatePresence>
          </div>
        )}
        {dockContributions
          .filter((c) => isDockOn(c.id))
          .map((c) => (
            <div
              key={`${c.pluginId}:${c.id}`}
              className="flex min-w-0 flex-1 flex-col overflow-hidden border-l border-borderSubtle"
            >
              <c.Component />
            </div>
          ))}
      </div>
    </div>
  );
};
