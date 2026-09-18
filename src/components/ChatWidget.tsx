import React, { useEffect, useLayoutEffect, useRef, useState, useMemo, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import ChatMessageList from './ChatMessageList';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { openProfilePopup } from '../utils/openProfilePopup';
import { Pickaxe, Zap, BarChart3, Filter, SquareSlash } from 'lucide-react';
import ChatSearchBar from './chat/ChatSearchBar';
import AutomodQueueStrip from './chat/AutomodQueueStrip';
import { useStreamerMode } from '../utils/streamerMode';
import { resolveUploadTarget, encodeExtraFields } from '../utils/imageUploadHosts';
import { registerChatSearchController } from '../keybindings/chatSearchController';
import { Dropdown, type DropdownOption } from './ui/Dropdown';

// Channel Points Icon (Twitch style)
const ChannelPointsIcon = ({ className = "", size = 14 }: { className?: string; size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" className={className} fill="currentColor">
    <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
    <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
  </svg>
);
import { useTwitchChat, type ModerationContext } from '../hooks/useTwitchChat';
import { useChannelEmotes, ensureChannelEmotes, getChannelEmotes, emoteCacheKey, refreshChannelEmotes, useChannelChat, useChannelChatMeta, useChatConnectionStore, setChannelPaused, injectRedemptionMessage, injectSystemMessage, systemSourceFor } from '../stores/chatConnectionStore';
import { useSpellcheck } from '../hooks/useSpellcheck';
import { warmSpellcheck } from '../utils/spellcheck';
import SpellcheckUnderlay from './chat/SpellcheckUnderlay';
import { makeKey } from '../utils/providerKey';
import { streamProvider } from '../utils/streamProvider';
import { PROVIDERS, type ProviderId } from '../types/providers';
import { PlatformAccountChip } from './PlatformAccountChip';
import { ProviderLogo } from './ProviderLogo';
import { useAppStore } from '../stores/AppStore';
import { useChannelState, watchChannel, unwatchChannel, refreshChannelState } from '../stores/channelStateStore';
import { useProviderEmoteStore } from '../stores/providerEmoteStore';
import { usePlatformAccountStore } from '../stores/platformAccountStore';
import { useFollowsStore } from '../stores/followsStore';
import { incrementStat } from '../services/supabaseService';
import { trackEmoteUsage } from '../utils/trackEmoteUsage';
import { LinkPreviewCard } from './chat/LinkPreviewCard';
import { extractPreviewUrls } from '../services/linkPreviewService';
import UserProfileCard from './UserProfileCard';
import ErrorBoundary from './ErrorBoundary';
import PredictionOverlay from './PredictionOverlay';
import PollOverlay from './PollOverlay';
import { ChatOverlayStack } from './chat/ChatOverlayStack';
import PollPredictionComposer from './chat/PollPredictionComposer';
import HypeTrainBanner from './HypeTrainBanner';
import ViewersPanel from './ViewersPanel';
import ModRoomPane from './modroom/ModRoomPane';
import { EmotePickerPanel, useSwappingSmiley } from './chat/EmotePickerPanel';
import { isCachedModerator, setCachedModerator, loadModeratedChannelIds, subscribeModeratedChannels } from '../services/modRoomService';
import type { ModRoomMember } from '../services/modRoomService';
import { ensureRoom, releaseRoom } from '../services/modRoomManager';
import { useModRoomStore } from '../stores/modRoomStore';
import ChannelPointsMenu from './ChannelPointsMenu';
import ModeratorMenu from './chat/ModeratorMenu';
import ResubNotificationBanner, { ResubNotification } from './ResubNotificationBanner';
import WatchStreakBanner, { WatchStreakMilestone } from './WatchStreakBanner';
import { Emote, EmoteSet, queueChannelEmotesForCaching, setEmoteCacheBurst } from '../services/emoteService';
import { prefetchChannelBadges } from '../services/badgeService';
import { parseBadges } from '../services/twitchBadges';
import { initializeBadgeImageCache } from '../services/badgeImageCacheService';
import { parseMessage } from '../services/twitchChat';
import {
  loadFavoriteEmotes,
  getAvailableFavorites
} from '../services/favoriteEmoteService';
import { getAppleEmojiUrl } from '../services/emojiService';
import { useChatUserStore } from '../stores/chatUserStore';
import { forceRefreshCosmetics } from '../services/cosmeticsCache';
import MentionAutocomplete from './MentionAutocomplete';
import CommandAutocomplete from './chat/CommandAutocomplete';
import CommandMenu from './chat/CommandMenu';
import EmoteAutocomplete from './chat/EmoteAutocomplete';
import SendAsPicker from './SendAsPicker';
import { useSendAccountStore } from '../stores/sendAccountStore';
import { getWordRange, EmoteTabCandidate } from '../utils/chatInputWord';
import {
  COMMAND_DEFINITIONS,
  CommandDefinition,
  buildUserCommandDefinitions,
  matchPlainTextUserCommand,
  expandUserCommand,
  findUserCommand,
} from '../utils/chatCommands';
import { buildTemplateContext, handleSlashCommand } from '../utils/commandHandler';
import { getRemindFlowSuggestions, tokenizeRemindOverlay } from '../utils/reminderEngine';

import { BackendChatMessage } from '../services/twitchChat';
import { registerChatModController, getChatModController, type ChatModController } from '../keybindings';
import { Tooltip } from './ui/Tooltip';

interface ParsedMessage {
  username: string;
  content: string;
  color: string;
  badges: Array<{ key: string; info: any }>;
  tags: Map<string, string>;
  emotes: string;
}

import { usemultiNookStore } from '../stores/multiNookStore';
import { usePinStore } from '../stores/pinStore';
import { useVodReplayStore, useVodReplaySnapshot, nudgeVodReplay } from '../stores/vodReplayStore';
import { SegmentedSelect } from './settings/_primitives';
import type { TwitchStream, HypeTrainData } from '../types';

import { Logger } from '../utils/logger';

// Unsent composer text per channel key, for the per-channel draft restore.
const chatDrafts = new Map<string, string>();
import { useVisibleInterval } from '../utils/useVisibleInterval';
import { formatUptimeClock } from '../utils/streamStats';
import { kickAppliedSeconds, kickTimeoutMinutes } from '../utils/kickTimeout';

// Channel Points hover tooltip — portalled to document.body to escape overflow-hidden
const ChannelPointsTooltip = ({ anchorRef, customPointsIconUrl, customPointsName, isLoadingChannelPoints, channelPoints }: {
  anchorRef: React.RefObject<HTMLDivElement | null>;
  customPointsIconUrl: string | null;
  customPointsName: string | null;
  isLoadingChannelPoints: boolean;
  channelPoints: number | null;
}) => {
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  // Horizontal offset applied to the bubble after viewport-edge clamping.
  // The arrow stays pinned to the anchor center, so when we shift the bubble
  // to keep it on-screen (e.g. inside the narrow MultiChat popout where the
  // anchor sits close to the right edge), the arrow's `left` is recomputed
  // separately so it still points at the icon below.
  const [bubbleShift, setBubbleShift] = useState(0);
  const bubbleRef = useRef<HTMLDivElement>(null);

  // Step 1: measure the anchor and stage the centered position. The bubble
  // isn't rendered yet on this pass (we early-return below when `pos` is null),
  // so we can't measure its width here.
  useLayoutEffect(() => {
    const el = anchorRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos({ left: rect.left + rect.width / 2, top: rect.top - 8 });
  }, [anchorRef]);

  // Step 2: now that the bubble is in the DOM (pos is set), measure its
  // natural width and clamp horizontally so it never escapes the viewport.
  // 8px margin keeps a breathing buffer at each edge. Re-runs whenever the
  // bubble's content can change width (custom points name, points number,
  // loading state) since those swap the rendered text.
  useLayoutEffect(() => {
    if (!pos) return;
    const bubble = bubbleRef.current;
    if (!bubble) return;
    const margin = 8;
    const bubbleRect = bubble.getBoundingClientRect();
    const half = bubbleRect.width / 2;
    const minLeft = margin + half;
    const maxLeft = window.innerWidth - margin - half;
    const clamped = Math.max(minLeft, Math.min(pos.left, maxLeft));
    setBubbleShift(clamped - pos.left);
  }, [pos, customPointsName, channelPoints, isLoadingChannelPoints]);

  if (!pos) return null;

  return createPortal(
    <div
      ref={bubbleRef}
      className="fixed px-3 py-1.5 bg-black/95 border border-border rounded-lg shadow-lg z-[9999] min-w-max pointer-events-none"
      style={{
        left: pos.left + bubbleShift,
        top: pos.top,
        transform: 'translate(-50%, -100%)',
      }}
    >
      <div className="flex items-center gap-1.5">
        {customPointsIconUrl ? (
          <img
            src={customPointsIconUrl}
            alt={customPointsName || "Channel Points"}
            className="w-[14px] h-[14px] flex-shrink-0"
          />
        ) : (
          <ChannelPointsIcon size={14} className="text-accent-neon flex-shrink-0" />
        )}
        {isLoadingChannelPoints ? (
          <span className="text-sm text-textSecondary">Loading...</span>
        ) : channelPoints !== null ? (
          <span className="text-sm font-bold text-accent-neon">{channelPoints.toLocaleString()}</span>
        ) : (
          <span className="text-sm text-textSecondary">--</span>
        )}
        {customPointsName && channelPoints !== null && (
          <span className="text-xs text-textSecondary">{customPointsName}</span>
        )}
      </div>
      {/* Arrow still points at the anchor center even when the bubble has
          been shifted to stay on-screen — its `left` is the anchor center
          relative to the (shifted) bubble's left edge. */}
      <div
        className="absolute top-full w-0 h-0 border-l-4 border-r-4 border-t-4 border-transparent border-t-black/95"
        style={{ left: `calc(50% - ${bubbleShift}px)`, transform: 'translateX(-50%)' }}
      />
    </div>,
    document.body
  );
};

/** Lets ChatWidget render against a caller-supplied channel instead of the
 *  AppStore's currentStream — same pattern as the MultiNook synthesis branch
 *  below, but driven by a prop rather than a separate store. Used by the
 *  StreamNook MultiChat popout window. When this prop is set it takes
 *  precedence over both `currentStream` and the MultiNook active slot.
 *
 *  Required fields (`user_login`, `user_id`) are chat-essential. Everything
 *  else is optional; the popout polls Helix for the rest (viewer count,
 *  uptime, title, game) and threads it through so the stream-view chrome
 *  (header counters, About panel, etc.) renders real values, not zeros. */
export interface ChatWidgetChannelOverride {
  user_login: string;
  user_id: string;
  user_name?: string;
  title?: string;
  game_name?: string;
  viewer_count?: number;
  started_at?: string;
  thumbnail_url?: string;
  profile_image_url?: string;
  broadcaster_type?: string;
  is_live?: boolean;
  /** MultiChat only: whether this pane is the active/focused one. The active pane
   *  owns the keyboard-moderation controller (so the popout's hotkeys act on the
   *  pane you're looking at, not every pane at once). */
  is_active?: boolean;
  /** Source platform. Absent/twitch = the full native Twitch path (unchanged).
   *  A non-twitch provider reads the shared `provider:channel` chat slice and
   *  gates off every Twitch-only behavior (Helix polls, points, mod, emotes). */
  provider?: ProviderId;
  /** Which surface this widget is mounted on. Absent means MultiChat (the only
   *  caller that passed an override before the main window could watch provider
   *  streams). 'main' keeps the surface-level affordances that MultiChat hosts
   *  elsewhere — notably the platform connect prompt, which in a popout lives in
   *  the Connections settings tab instead. */
  context?: 'main' | 'multichat';
}

export interface ChatWidgetProps {
  channelOverride?: ChatWidgetChannelOverride;
  /** MultiChat only: the per-pane hype train (polled by MultiChatPane). The main
   *  app leaves this unset and uses the global store value instead. */
  hypeTrainOverride?: HypeTrainData | null;
  /** Saved message filter (settings.chat_query.filters id) this pane starts
   *  on. The pane owns the live value; MultiChat persists it per tab. */
  filterId?: string | null;
  onFilterIdChange?: (id: string | null) => void;
}

// Minimum spacing between pause/resume transitions. Real gestures are hundreds
// of ms apart, so this is invisible to users, but it caps machine-speed
// scroll/auto-scroll oscillation so it can never reach React's 50-deep render
// limit. `force` transitions (channel switch, navigation, Resume button) bypass
// it. See `setChatPaused` in ChatWidget.
const PAUSE_SETTLE_MS = 120;

// Structurally complete, empty, frozen. YouTube has no third-party emote fetch,
// so `useChannelEmotes` returns null for it forever, but the picker still needs
// a real EmoteSet to merge the harvested `youtube` slot into. Without a base
// object the merge below short-circuits and every tab renders zero.
const EMPTY_EMOTE_SET = Object.freeze({
  twitch: [],
  bttv: [],
  '7tv': [],
  ffz: [],
  kick: [],
  youtube: [],
}) as unknown as EmoteSet;

/** A follower-mode minimum, in minutes, as something readable.
 *  Kick states these in minutes and they get large — xqc's is 5760, which as
 *  "5760m" is a number you have to do arithmetic on to understand. */
function formatFollowAge(minutes: number): string {
  if (minutes >= 10080 && minutes % 10080 === 0) return `${minutes / 10080}w`;
  if (minutes >= 1440) {
    const days = Math.round(minutes / 1440);
    return `${days}d`;
  }
  if (minutes >= 60) {
    const hours = Math.round(minutes / 60);
    return `${hours}h`;
  }
  return `${minutes}m`;
}

// C7 checkpoint B: the message-list subtree owns its own per-channel chat
// subscription. While the parent still holds its own subscription this is
// behavior-identical; checkpoint C cuts the parent's per-flush subscription so
// only this panel re-renders on message traffic. Replay mode has no live
// slice, so the parent passes the replay snapshot through as an override.
interface ChatMessagesPanelSource {
  messages: (string | BackendChatMessage)[];
  renderToken: number;
  deletedMessageIds: Set<string>;
  clearedUserContexts: Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>;
}

const USER_HISTORY_MAX_USERS = 300;

interface ChatMessagesPanelProps {
  /** Live slice key (bare Twitch login / composite provider key); ignored when
   *  an override snapshot is supplied. */
  channelKey: string | null;
  provider: ProviderId;
  providerKey: string | null;
  stream: TwitchStream | null | undefined;
  kickAccountName: string | null;
  onKickModeratorDetected: () => void;
  setIsSharedChat: (v: boolean) => void;
  userMessageHistoryRef: React.RefObject<Map<string, ParsedMessage[]>>;
  sharedRoomsRef: React.RefObject<Set<string>>;
  processedIdsRef: React.RefObject<Set<string>>;
  messagesRef: React.RefObject<(string | BackendChatMessage)[]>;
  /** Replay-mode snapshot; null in live mode. */
  override: ChatMessagesPanelSource | null;
  isPaused: boolean;
  onPauseIntent: () => void;
  onScroll: (distanceToBottom: number, isUserScroll: boolean) => void;
  onUsernameClick: React.ComponentProps<typeof ChatMessageList>['onUsernameClick'];
  onReplyClick: (parentMsgId: string) => void;
  onMessageCopy?: (content: string) => void;
  onEmoteRightClick: (emoteName: string) => void;
  onUsernameRightClick: (messageId: string, username: string) => void;
  onBadgeClick: (badgeKey: string, badgeInfo: Record<string, unknown>) => void;
  highlightedMessageId: string | null;
  modFocusId?: string | null;
  hiddenMessageIds?: Set<string>;
  emotes: EmoteSet | null;
  getMessageId: (message: string | BackendChatMessage) => string | null;
  isModerator?: boolean;
  broadcasterId?: string;
  hoveringRef: React.RefObject<boolean>;
  /** Saved filter id: show only rows the Rust rule engine stamped with it. */
  filterId?: string | null;
}

/** How many consecutive untagged messages end the shared-chat indicator. */
const SHARED_CHAT_END_STREAK = 10;

const ChatMessagesPanel = ({
  channelKey,
  provider,
  providerKey,
  stream,
  kickAccountName,
  onKickModeratorDetected,
  setIsSharedChat,
  userMessageHistoryRef,
  sharedRoomsRef,
  processedIdsRef,
  messagesRef,
  override,
  isPaused,
  onPauseIntent,
  onScroll,
  onUsernameClick,
  onReplyClick,
  onMessageCopy,
  onEmoteRightClick,
  onUsernameRightClick,
  onBadgeClick,
  highlightedMessageId,
  modFocusId,
  hiddenMessageIds,
  emotes,
  getMessageId,
  isModerator,
  broadcasterId,
  hoveringRef,
  filterId,
}: ChatMessagesPanelProps) => {
  const live = useChannelChat(override ? null : channelKey);
  const src: ChatMessagesPanelSource = override ?? live;
  // A saved filter narrows this pane to rows the Rust rule engine stamped
  // with its id (metadata.filter_ids). Render-time only, so switching the
  // filter never drops history; O(buffer) per flush, and the buffer is capped.
  const shownMessages = useMemo(() => {
    if (!filterId) return src.messages;
    return src.messages.filter(
      (m) => typeof m !== 'string' && !!m.metadata?.filter_ids?.includes(filterId),
    );
  }, [src.messages, filterId]);
  const addUser = useChatUserStore((state) => state.addUser);
  /** Consecutive Twitch messages seen with no source-room-id tag. Drives the
   *  shared-chat session-end detection below. */
  const sharedChatQuietRef = useRef(0);

  // Keyboard moderation reads the visible messages synchronously through this
  // ref; the panel owns the snapshot now, so it keeps the ref fresh.
  useEffect(() => {
    messagesRef.current = src.messages;
  }, [src.messages, src.renderToken, messagesRef]);

  // Process new messages for user history tracking.
  //
  // Extract the message id CHEAPLY (regex on raw IRC tags, or the object's
  // `id`) and check processedMessageIdsRef BEFORE calling the far more
  // expensive parseMessage. Parsing the whole capped array on every render
  // stalls the main thread in a fast chat, since almost all of it is already
  // processed.
  useEffect(() => {
    const seen = processedIdsRef.current;
    const currentIds = new Set<string>();

    for (const message of src.messages) {
      // Cheap ID extraction first, no full parse.
      let msgId: string | undefined;
      if (typeof message === 'string') {
        const m = message.match(/(?:^@|;)id=([^;\s]+)/);
        msgId = m ? m[1] : undefined;
      } else {
        msgId = message.id;
      }

      if (msgId) {
        currentIds.add(msgId);
        if (seen.has(msgId)) continue; // Already processed — skip the parse + side effects.
        seen.add(msgId);
      }

      try {
        let parsed: ParsedMessage;
        let userId: string | undefined;
        let username: string | undefined;
        let displayName: string | undefined;
        let userColor: string | undefined;

        if (typeof message === 'string') {
          const channelIdMatch = message.match(/room-id=([^;]+)/);
          const channelId = channelIdMatch ? channelIdMatch[1] : undefined;
          parsed = parseMessage(message, channelId);
          userId = parsed.tags.get('user-id');
          username = parsed.username;
          displayName = parsed.tags.get('display-name') || parsed.username;
          userColor = parsed.color;
        } else {
          // Backend message object
          parsed = parseMessage(message);
          userId = message.tags['user-id'] || message.user_id;
          username = message.username;
          displayName = message.display_name || message.username;
          userColor = message.color || parsed.color;
        }

        // YouTube ships each custom emoji inline with the message that uses it and
        // offers no set to fetch, so the picker learns them here. Segments come off
        // the BACKEND message (Rust pre-parses them); `parsed` doesn't carry them.
        if (provider === 'youtube' && providerKey && typeof message !== 'string') {
          const segments = message.segments;
          if (segments?.length) {
            useProviderEmoteStore.getState().harvest(providerKey, segments);
          }
        }

        // Shared-chat detection, incremental: a cross-room message flips the
        // sticky flag and prefetches the source room's badge metadata ONCE
        // (subscriber/bits/founder badges render blank without that prefetch).
        if (provider === 'twitch') {
          const srcRoom =
            typeof message === 'string'
              ? parsed.tags.get('source-room-id')
              : message.tags['source-room-id'];
          const ownRoom =
            typeof message === 'string'
              ? parsed.tags.get('room-id')
              : message.tags['room-id'];
          if (srcRoom && ownRoom && srcRoom !== ownRoom) {
            sharedChatQuietRef.current = 0;
            setIsSharedChat(true);
            if (!sharedRoomsRef.current.has(srcRoom)) {
              sharedRoomsRef.current.add(srcRoom);
              prefetchChannelBadges(srcRoom).catch((err) =>
                Logger.warn('[ChatWidget] Failed to prefetch badges for shared channel:', srcRoom, err),
              );
            }
          } else if (srcRoom) {
            // Still in the session, just a message from this channel itself.
            sharedChatQuietRef.current = 0;
          } else if (++sharedChatQuietRef.current >= SHARED_CHAT_END_STREAK) {
            // Twitch tags EVERY message in a shared-chat session with source-room-id,
            // including the host channel's own, so messages arriving without it mean the
            // session ended. Without this the flag stays set until the next channel
            // change.
            //
            // A streak rather than a single miss: the docs describe the tag's presence
            // without guaranteeing it on every message shape, and one stray untagged line
            // must not flicker the indicator.
            setIsSharedChat(false);
          }
        }

        if (
          provider !== 'twitch' &&
          kickAccountName &&
          typeof message !== 'string' &&
          ((message.username as string) || '').toLowerCase() === kickAccountName
        ) {
          const mBadges = (message.badges as Array<{ name?: string }> | undefined) || [];
          if (mBadges.some((b) => b.name === 'moderator' || b.name === 'broadcaster')) {
            onKickModeratorDetected();
          }
        }

        if (userId) {
          // Namespaced for non-Twitch. `message.user_id` is a RAW platform id, so
          // Kick user 676 and Twitch user 676 are different people who would
          // otherwise share one history bucket and show each other's messages.
          const historyKey = provider === 'twitch' ? userId : `${provider}:${userId}`;
          const history = userMessageHistoryRef.current.get(historyKey) || [];
          history.push(parsed);
          if (history.length > 50) history.shift();
          // Delete-then-set keeps Map insertion order as a recency order, so
          // the eviction below always drops the longest-silent chatter.
          userMessageHistoryRef.current.delete(historyKey);
          userMessageHistoryRef.current.set(historyKey, history);
          if (userMessageHistoryRef.current.size > USER_HISTORY_MAX_USERS) {
            const oldest = userMessageHistoryRef.current.keys().next().value;
            if (oldest !== undefined) userMessageHistoryRef.current.delete(oldest);
          }

          // Add user to mention autocomplete store. Channel context drives
          // third-party badge resolution inside the store.
          if (username && displayName) {
            const channelId =
              parsed.tags.get('source-room-id') ||
              parsed.tags.get('room-id') ||
              stream?.user_id ||
              '';
            const channelName =
              stream?.user_login ||
              stream?.user_name ||
              parsed.tags.get('room') ||
              '';
            addUser(
              {
                // Namespace non-Twitch chatters so their 7TV cosmetics resolve
                // under the right platform and never collide with a Twitch id of
                // the same number. Twitch stays the bare id (byte-identical). This
                // matches ChatMessage's `cosmeticsKey`.
                userId: provider === 'twitch' ? userId : `${provider}:${userId}`,
                username,
                displayName,
                color: userColor || '#9147FF',
              },
              channelId ? { channelId, channelName } : undefined,
            );
          }
        }
      } catch (err) {
        Logger.error('[ChatWidget] Failed to parse message:', err, message);
      }
    }

    // Drop processed-IDs for messages that have rolled out of the array.
    // Without this the set would grow unbounded across the session.
    for (const id of seen) {
      if (!currentIds.has(id)) seen.delete(id);
    }
    // `renderToken` stays in the deps alongside `messages`. The store writes a
    // fresh array per change, so identity alone would cover the Twitch path, but
    // the token also covers provider snapshots and any future in-place writer.
    // A missed run means chatters never reach the chat-user store and never
    // resolve 7TV cosmetics. Re-running per flush is cheap: every message already
    // handled is skipped, so each new one is parsed exactly once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [src.messages, src.renderToken, addUser]);

  return (
    <div
      className="flex-1 overflow-hidden animate-panel-slide-down"
      onMouseEnter={() => { hoveringRef.current = true; }}
      onMouseLeave={() => { hoveringRef.current = false; }}
    >
      {src.messages.length === 0 ? (
        <div className="h-full flex items-center justify-center">
          <p className="text-textSecondary text-sm">Waiting for messages...</p>
        </div>
      ) : (
        <ErrorBoundary componentName="ChatWidgetList" reportToLogService={true}>
          <ChatMessageList
            messages={shownMessages}
            renderToken={src.renderToken}
            isPaused={isPaused}
            onPauseIntent={onPauseIntent}
            onScroll={onScroll}
            onUsernameClick={onUsernameClick}
            onReplyClick={onReplyClick}
            onMessageCopy={onMessageCopy}
            onEmoteRightClick={onEmoteRightClick}
            onUsernameRightClick={onUsernameRightClick}
            onBadgeClick={onBadgeClick}
            highlightedMessageId={highlightedMessageId}
            modFocusId={modFocusId}
            deletedMessageIds={src.deletedMessageIds}
            hiddenMessageIds={hiddenMessageIds}
            clearedUserContexts={src.clearedUserContexts}
            emotes={emotes}
            getMessageId={getMessageId}
            isModerator={isModerator}
            broadcasterId={broadcasterId}
          />
        </ErrorBoundary>
      )}
    </div>
  );
};

// "N new since paused": anchored to the channel's MONOTONIC live-message
// counter at the instant pause begins (accurate under buffer trimming, unlike
// a messages.length diff). Isolated here so its per-flush subscription
// re-renders only this span, never the widget.
const PausedNewCount = ({ channelKey, isPaused }: { channelKey: string | null; isPaused: boolean }) => {
  const key = channelKey ? channelKey.toLowerCase() : null;
  useChatConnectionStore((state) => (key && isPaused ? state.revisionByChannel[key] ?? 0 : 0));
  // Anchor captured once per pause via effect (never a render-time ref write).
  const [anchor, setAnchor] = useState<number | null>(null);
  useEffect(() => {
    if (!isPaused || !key) {
      setAnchor(null);
      return;
    }
    setAnchor(
      (a) => a ?? useChatConnectionStore.getState().channels.get(key)?.liveMessageCount ?? 0,
    );
  }, [isPaused, key]);
  if (!isPaused || !key || anchor === null) return null;
  const liveCount = useChatConnectionStore.getState().channels.get(key)?.liveMessageCount ?? 0;
  const delta = Math.max(0, liveCount - anchor);
  return delta > 0 ? <> ({delta} new)</> : null;
};

const ChatWidget = ({ channelOverride, hypeTrainOverride, filterId: filterIdProp, onFilterIdChange }: ChatWidgetProps = {}) => {
  // Single source of truth for the source platform. Twitch (the default) runs the
  // entire native path below unchanged; a non-twitch provider reads the shared
  // `provider:channel` slice and every Twitch-only effect early-returns on it.
  const provider: ProviderId = channelOverride?.provider ?? 'twitch';
  const isTwitch = provider === 'twitch';
  // The main window, whether watching Twitch (no override) or a provider stream
  // (override tagged `context: 'main'`). MultiChat popouts are the other case.
  const isMainSurface = !channelOverride || channelOverride.context === 'main';

  // Message-source seam (the only structural change). Both hooks ALWAYS run
  // (rules-of-hooks); we select by provider. Twitch -> `chat` IS the existing
  // useTwitchChat() result, byte-identical. A provider -> read its already-
  // connected slice (the MultiChat add-source flow connected it) and stub the
  // Twitch-only actions (connect is a no-op; send is read-only until OAuth).
  const twitchChat = useTwitchChat();
  const providerKey =
    !isTwitch && channelOverride ? makeKey(provider, channelOverride.user_login.toLowerCase()) : null;
  const providerMeta = useChannelChatMeta(providerKey);
  // Hoisted out of the memo below so their identities survive a flush. The memo
  // now recomputes on every renderToken bump, so functions declared inline in it
  // would churn per frame and destabilize everything downstream (setChatPaused
  // depends on setPaused, and the message list's handlers depend on that).
  const providerConnectChat = useCallback(async () => {}, []);
  const providerSendMessage = useCallback(
    async (
      messageText: string,
      _userInfo?: unknown,
      replyParentMsgId?: string,
      _senderAccount?: unknown,
    ) => {
      if (!channelOverride) return;
      let text = messageText;
      let replyTo = replyParentMsgId ?? null;
      // YouTube live chat has no reply threads, so a reply becomes the @mention
      // YouTube's own client would send. Resolved here because the backend keeps
      // no parent-message cache to look the display name up in.
      if (provider === 'youtube' && replyParentMsgId) {
        const sliceMsgs = providerKey
          ? useChatConnectionStore.getState().channels.get(providerKey)?.messages ?? []
          : [];
        const parent = sliceMsgs.find(
          (m) => typeof m !== 'string' && (m as BackendChatMessage).id === replyParentMsgId,
        ) as BackendChatMessage | undefined;
        const parentName = parent?.display_name || parent?.username;
        if (parentName) text = `@${parentName} ${text}`;
        replyTo = null;
      }
      const outcome = await invoke<{
        message_id: string | null;
        is_sent: boolean;
        drop_reason: string | null;
      }>('provider_send_message', {
        provider,
        channel: channelOverride.user_login.toLowerCase(),
        text,
        replyTo,
      });
      // The platform can accept the request and still refuse the message (timeout,
      // slow mode, banned word). Throwing routes into the caller's catch, which
      // restores the composer text and surfaces the reason.
      if (outcome && outcome.is_sent === false) {
        throw new Error(outcome.drop_reason || 'Message not sent');
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [provider, channelOverride, providerKey],
  );
  const providerSetPaused = useCallback(
    (paused: boolean) => {
      if (providerKey) setChannelPaused(providerKey, paused);
    },
    [providerKey],
  );
  // Meta + actions only: the panel owns the per-flush snapshot now, so the
  // provider path no longer routes messages through the parent at all.
  const providerChat = useMemo(
    () => ({
      connectChat: providerConnectChat,
      sendMessage: providerSendMessage,
      isConnected: providerMeta.isConnected,
      error: providerMeta.error,
      setPaused: providerSetPaused,
      roomState: providerMeta.roomState,
      userBadges: providerMeta.userBadges,
    }),
    [
      providerMeta.isConnected,
      providerMeta.error,
      providerMeta.roomState,
      providerMeta.userBadges,
      providerConnectChat,
      providerSendMessage,
      providerSetPaused,
    ],
  );
  // VOD chat replay seam. In the main widget (never a popout) while a VOD is
  // playing, the panel defaults to synced historical chat and can toggle to the
  // channel's live chat. The replay hook always runs (rules of hooks); we just
  // select it as the message source when in replay mode. Replay is read-only.
  // Read before the replay seam below, which gates on it.
  const currentMediaType = useAppStore((s) => s.currentMediaType);
  const replayChat = useVodReplaySnapshot();
  const replaySessionId = useVodReplayStore((s) => s.sessionId);
  const replayActive = useVodReplayStore((s) => s.active);
  // The toggle is available whenever a VOD replay session exists — started for a
  // VOD opened from the Videos list OR via the offline-channel button. Never in
  // popouts (they always show their own live channel), and never over a LIVE
  // room: the store flag is cleared by startStream, but if a session ever
  // outlives its media again, the media type is the fact that cannot lie.
  const isVodReplay =
    !channelOverride &&
    replayActive &&
    (currentMediaType === 'video' || currentMediaType === 'clip' || currentMediaType === 'offline_chat');
  const [chatMode, setChatMode] = useState<'replay' | 'live'>('replay');
  const chat = isVodReplay && chatMode === 'replay' ? replayChat : isTwitch ? twitchChat : providerChat;
  const { connectChat, sendMessage, isConnected, setPaused: setBufferPaused, roomState, userBadges } = chat;

  // A new VOD always starts in replay (beginVodReplay bumps sessionId).
  useEffect(() => {
    setChatMode('replay');
  }, [replaySessionId]);

  // Returning to replay from live: catch up to the current playhead instantly.
  // The engine keeps running across the toggle, so this never resets position —
  // it just avoids waiting up to one poll interval to resync.
  useEffect(() => {
    if (isVodReplay && chatMode === 'replay') nudgeVodReplay();
  }, [isVodReplay, chatMode]);

  // Kick sending requires a connected Kick account (OAuth). Poll the state so the
  // composer enables right after the user connects.
  // Connection state comes from the shared store, which is event-driven. This
  // used to be a 5s `kick_is_connected` poll PER PANE, reading a pure in-memory
  // bool that only ever changes when the user connects or disconnects.
  const kickConnected = usePlatformAccountStore((s) => provider === 'kick' && s.kick.connected);
  // The connected Kick account's username, lowercased — used to spot our OWN
  // (badged) messages so we can tell whether we may moderate this Kick channel
  // (Kick gives no Twitch-style USERSTATE with our role).
  const kickAccountName = usePlatformAccountStore((s) =>
    provider === 'kick' && s.kick.connected ? (s.kick.name?.toLowerCase() ?? null) : null,
  );
  // YouTube sending + moderation drive the webview-session login. Poll connection so
  // the composer enables right after connecting.
  const youtubeConnected = usePlatformAccountStore(
    (s) => provider === 'youtube' && s.youtube.connected,
  );
  const connectPlatformAccount = usePlatformAccountStore((s) => s.connect);
  // YouTube exposes no "are you a mod" flag, so the backend probes a message's
  // context menu. This one is a REAL network probe, not a local read, so it stays
  // a poll — the answer depends on a message having been seen and on the probe
  // succeeding. Visibility-gated now, so a backgrounded window stops asking; the
  // backend also rate-limits a failing probe to once a minute.
  const [youtubeCanModerate, setYoutubeCanModerate] = useState(false);
  const youtubeSlug = channelOverride?.user_login;
  const canModerateActive = provider === 'youtube' && youtubeConnected && !!youtubeSlug;
  useEffect(() => {
    if (canModerateActive) return;
    setYoutubeCanModerate(false);
  }, [canModerateActive]);
  useEffect(() => {
    if (!canModerateActive) return;
    let active = true;
    invoke<boolean>('youtube_can_moderate', { channel: youtubeSlug })
      .then((c) => {
        if (active) setYoutubeCanModerate(c);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [canModerateActive, youtubeSlug]);
  useVisibleInterval(() => {
    if (!canModerateActive) return;
    invoke<boolean>('youtube_can_moderate', { channel: youtubeSlug })
      .then(setYoutubeCanModerate)
      .catch(() => {});
  }, 8000);
  // Kick's authoritative mod check. The badge heuristic below can only learn our
  // role once we have SPOKEN in the channel, so a moderator who is only watching
  // had no mod controls at all. Deliberately NOT polled: unlike YouTube's probe
  // (which depends on a message having been seen), this asks Kick directly and
  // the answer is near-static, and the heuristic still covers a mid-session
  // promotion. The backend caches for 5 minutes, negatives included.
  const [kickViewer, setKickViewer] = useState<{
    can_moderate: boolean;
    following_since: string | null;
    subscribed_for: number;
  } | null>(null);
  const kickCanModerate = kickViewer ? kickViewer.can_moderate : null;
  const kickSlug = provider === 'kick' ? channelOverride?.user_login?.toLowerCase() : undefined;
  const kickModActive = provider === 'kick' && kickConnected && !!kickSlug;
  useEffect(() => {
    if (kickModActive) return;
    setKickViewer(null);
  }, [kickModActive]);
  useEffect(() => {
    if (!kickModActive) return;
    let active = true;
    invoke<{ can_moderate: boolean; following_since: string | null; subscribed_for: number }>(
      'kick_viewer_state',
      { channel: kickSlug },
    )
      .then((v) => {
        if (active) setKickViewer(v);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [kickModActive, kickSlug]);
  // Field selectors instead of whole-store subscriptions: ChatWidget re-renders
  // only when these specific fields change, not on every unrelated store tick.
  const rawCurrentStream = useAppStore((s) => s.currentStream);
  const currentUser = useAppStore((s) => s.currentUser);
  const openEmoteSets = useAppStore((s) => s.openEmoteSets);
  const externalDropsProvider = useAppStore((s) => s.externalDropsProvider);
  const globalHypeTrain = useAppStore((s) => s.currentHypeTrain);
  // In MultiChat the pane drives its own hype train (via the prop); the main app
  // uses the global store value. Resolving it here means the banner, countdown
  // and level-up logic below all work per-pane with no further changes.
  const currentHypeTrain = channelOverride ? (hypeTrainOverride ?? null) : globalHypeTrain;
  const isMultiNookActive = usemultiNookStore((s) => s.isMultiNookActive);
  const activeChatChannelId = usemultiNookStore((s) => s.activeChatChannelId);
  const slots = usemultiNookStore((s) => s.slots);

  const currentStream = useMemo(() => {
    // Popout (StreamNook MultiChat) channel takes priority. Synthesizes a
    // TwitchStream from caller-supplied metadata; the popout polls Helix for
    // viewer count / uptime / game / title and threads them through here.
    if (channelOverride) {
      return {
        id: channelOverride.user_id,
        user_id: channelOverride.user_id,
        user_login: channelOverride.user_login,
        user_name: channelOverride.user_name || channelOverride.user_login,
        game_id: '',
        game_name: channelOverride.game_name ?? '',
        type: channelOverride.is_live === false ? '' : 'live',
        title: channelOverride.title ?? '',
        viewer_count: channelOverride.viewer_count ?? 0,
        // Empty (not `new Date()`) when the start time is unknown: faking "now"
        // made the uptime tick from when you opened the chat (a fake local start)
        // and flip to that whenever a meta poll briefly lacked started_at. Empty
        // just hides the uptime until the real start time is known.
        started_at: channelOverride.started_at ?? '',
        language: 'en',
        thumbnail_url: channelOverride.thumbnail_url ?? '',
        profile_image_url: channelOverride.profile_image_url,
        broadcaster_type: channelOverride.broadcaster_type,
        is_live: channelOverride.is_live ?? true,
        tag_ids: [],
        is_mature: false,
      } as TwitchStream;
    }
    if (isMultiNookActive && activeChatChannelId) {
      // activeChatChannelId is a composite provider key (makeKey convention;
      // absent slot provider means Twitch). A login-only match is ambiguous
      // once a grid can hold two platforms: a Kick and a Twitch tile can share
      // a login, and first-match would synthesize the wrong tile's identity.
      const activeSlot = slots.find(
        (s) => makeKey(s.provider ?? 'twitch', s.channelLogin) === activeChatChannelId,
      );
      if (activeSlot) {
        return {
          id: activeSlot.channelId || activeSlot.id,
          user_id: activeSlot.channelId || '',
          user_login: activeSlot.channelLogin,
          user_name: activeSlot.channelName || activeSlot.channelLogin,
          game_id: '',
          game_name: 'multi-nook',
          type: 'live',
          title: `multi-nook: ${activeSlot.channelName || activeSlot.channelLogin}`,
          viewer_count: 0,
          started_at: new Date().toISOString(),
          language: 'en',
          thumbnail_url: '',
          tag_ids: [],
          is_mature: false
        } as TwitchStream;
      }
    }
    return rawCurrentStream;
  }, [channelOverride, rawCurrentStream, isMultiNookActive, activeChatChannelId, slots]);

  // Kick has no USERSTATE, so derive our role from our OWN messages: if one of
  // them (matched by the connected Kick username) carries a moderator/broadcaster
  // badge, we can moderate this channel. Covers both the broadcaster and mods.
  // Kick has no USERSTATE; the panel's per-new-message loop spots our own
  // moderator/broadcaster badge and reports it once (sticky until the channel
  // changes). Equivalent to the old whole-buffer memo: the loop sees every
  // buffered and backfilled message exactly once.
  const [kickModDetected, setKickModDetected] = useState(false);
  const onKickModeratorDetected = useCallback(() => setKickModDetected(true), []);
  const isModerator = useMemo(() => {
    if (provider === 'youtube') return youtubeCanModerate;
    // The API answer wins once it arrives, but the badge heuristic stays as an OR:
    // a promotion made during this session shows up in our badges long before the
    // cached probe expires.
    if (provider === 'kick') {
      return kickCanModerate === null ? kickModDetected : kickCanModerate || kickModDetected;
    }
    if (!isTwitch) return false; // tiktok + other read-only providers: no mod actions
    if (!userBadges) return false;
    return userBadges.includes('moderator') || userBadges.includes('broadcaster');
  }, [provider, youtubeCanModerate, isTwitch, kickModDetected, kickCanModerate, userBadges]);
  
  // UI state
  const [messageInput, setMessageInput] = useState('');
  // Per-channel drafts: switching channels parks the unsent text and brings
  // back whatever was typed there before (module-level, survives remounts).
  const draftKeyRef = useRef<string | null>(null);
  // Slow-mode countdown: the wall-clock of our last send; the composer shows
  // the remaining wait while roomState.slow is on.
  const [lastSentAt, setLastSentAt] = useState(0);
  const [slowNow, setSlowNow] = useState(0);
  const [activeView, setActiveView] = useState<'chat' | 'viewers' | 'modroom'>('chat');
  // Streamer mode (Rust-owned flag): hides the viewer count here.
  const streamerModeActive = useStreamerMode((st) => st.active);
  useEffect(() => {
    useStreamerMode.getState().start();
  }, []);
  // Ctrl+F search bar over the Rust-owned history ring (chat/ChatSearchBar).
  const [searchOpen, setSearchOpen] = useState(false);
  const searchOpenRef = useRef(false);
  useEffect(() => {
    searchOpenRef.current = searchOpen;
  }, [searchOpen]);
  // Saved message filter bound to this pane (settings.chat_query.filters).
  const [filterId, setFilterIdState] = useState<string | null>(filterIdProp ?? null);
  useEffect(() => {
    setFilterIdState(filterIdProp ?? null);
  }, [filterIdProp]);
  const setFilterId = useCallback(
    (id: string | null) => {
      setFilterIdState(id);
      onFilterIdChange?.(id);
    },
    [onFilterIdChange],
  );
  const savedFilters = useAppStore((s) => s.settings.chat_query?.filters);
  const filterOptions = useMemo<DropdownOption<string>[]>(
    () => [
      { value: '', label: 'All messages' },
      ...(savedFilters ?? [])
        .filter((f) => f.enabled && f.expr.trim())
        .map((f) => ({ value: f.id, label: f.name || f.expr })),
    ],
    [savedFilters],
  );
  // Mod-room status reported up by ModRoomPane so the header can show it.
  const [modRoomStatus, setModRoomStatus] = useState<{
    memberCount: number;
    encrypted: boolean;
    connected: boolean;
    members: ModRoomMember[];
  }>({
    memberCount: 0,
    encrypted: false,
    connected: false,
    members: [],
  });

  // Optimistic mod-room eligibility: show the toggle instantly on revisit from a
  // per-channel cache while USERSTATE (isModerator) confirms or clears it. Safe to
  // be optimistic — the gate verifies mod status server-side.
  const [cachedMod, setCachedMod] = useState(false);
  useEffect(() => {
    setCachedMod(isCachedModerator(currentStream?.user_id));
  }, [currentStream?.user_id]);
  useEffect(() => {
    // Twitch only: a Kick/YouTube provider snapshot must never write its channel
    // id into the Twitch mod-room cache (the id namespaces collide).
    if (!isTwitch) return;
    const id = currentStream?.user_id;
    if (!id || !userBadges) return; // persist only once USERSTATE has resolved
    setCachedModerator(id, isModerator);
    setCachedMod(isModerator);
  }, [isTwitch, isModerator, userBadges, currentStream?.user_id]);
  // Mod rooms are Twitch-only: without the isTwitch gate, a Kick/YouTube
  // moderator in an override pane would get a Mods toggle whose channel id
  // belongs to the wrong platform.
  const modRoomEligible = isTwitch && (isModerator || cachedMod);

  // Keep the room connected while watching a moderated channel, independent of
  // which view is showing, so unread/mention badges work from the Chat view.
  useEffect(() => {
    const id = currentStream?.user_id;
    if (!id || !modRoomEligible) return;
    ensureRoom(id);
    return () => releaseRoom(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modRoomEligible, currentStream?.user_id]);

  // Unread/mention counts for the badge on the Mods toggle.
  const modRoomUnread = useModRoomStore((s) =>
    currentStream ? s.sessions[currentStream.user_id]?.unread ?? 0 : 0,
  );
  const modRoomMentions = useModRoomStore((s) =>
    currentStream ? s.sessions[currentStream.user_id]?.mentions ?? 0 : 0,
  );

  // Seed the moderated-channel cache once (if the scoped token exists) so the
  // toggle shows on first visit too, not only on revisit.
  useEffect(() => {
    loadModeratedChannelIds().then(() => setCachedMod(isCachedModerator(currentStream?.user_id)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // Re-check when the moderated list re-resolves (e.g. right after consent), so
  // the toggle appears for this channel without an app restart. Re-subscribes on
  // channel change to read the current channel's status.
  useEffect(() => {
    return subscribeModeratedChannels(() => setCachedMod(isCachedModerator(currentStream?.user_id)));
  }, [currentStream?.user_id]);

  // The viewers list is mod-only (Helix Get Chatters needs mod/broadcaster auth).
  // If mod status drops while it's open, fall back to the chat view. The mod-room
  // tab uses the optimistic eligibility so it isn't yanked before USERSTATE lands.
  useEffect(() => {
    if (activeView === 'viewers' && (!isModerator || !isTwitch)) setActiveView('chat');
    if (activeView === 'modroom' && !modRoomEligible) setActiveView('chat');
  }, [activeView, isModerator, modRoomEligible, isTwitch]);
  const [showEmotePicker, setShowEmotePicker] = useState(false);
  // Browsable command menu (button left of the emote picker). Mutually
  // exclusive with the emote picker: they share the space above the box.
  const [showCommandMenu, setShowCommandMenu] = useState(false);
  const commandMenuButtonRef = useRef<HTMLButtonElement>(null);
  // While the picker is open, fill the emote disk cache aggressively (the user is
  // actively waiting on these); the matching cleanup drops back to the polite
  // background trickle on close. Ref-counted in the service so split panes /
  // popouts compose without one close cutting another's burst short.
  useEffect(() => {
    if (!showEmotePicker) return;
    setEmoteCacheBurst(true);
    return () => setEmoteCacheBurst(false);
  }, [showEmotePicker]);
  // Multi-account "send as" picker. Only shown when 2+ accounts are linked, so a
  // single-account user sees no change. Load the registry once on mount.
  const linkedAccountCount = useSendAccountStore((s) => s.accounts.length);
  const showSendAsPicker = linkedAccountCount >= 2;
  useEffect(() => {
    useSendAccountStore.getState().loadAccounts();
  }, []);
  // Shared per-channel EmoteSet from chatConnectionStore. Multiple ChatWidget
  // instances rendering the same channel (e.g. split-mode columns or the
  // popout opened alongside the main app) hold a single reference instead
  // of each fetching + caching their own copy. Keyed strictly by lowercase
  // channel login so 7TV name collisions across channels stay isolated.
  const baseEmotes = useChannelEmotes(
    currentStream?.user_login ?? null,
    currentStream?.user_id ?? null,
    provider,
  );
  // YouTube's emoji are learned from chat rather than fetched (no set endpoint
  // exists), so they live in their own store and are folded in here. Subscribing
  // to the map for THIS channel only, so an unrelated channel's harvest can't
  // re-render this widget.
  const harvestedYouTube = useProviderEmoteStore((s) =>
    provider === 'youtube' && providerKey ? s.byChannel[providerKey] : undefined,
  );
  // Channels whose authoritative set we've already installed. Deliberately NOT
  // `byChannel[key].size`: message harvesting writes into the same map, so a
  // single emote posted before the seed landed would look like "already seeded"
  // and lock the picker to whatever chat happened to say.
  const seededYouTubeRef = useRef<Set<string>>(new Set());
  // The channel's REAL emoji set, from the live_chat page the adapter already
  // loads. Harvesting from messages only learns what somebody happened to post;
  // this is the whole list, and it seeds the store so the two merge.
  useEffect(() => {
    if (provider !== 'youtube' || !providerKey) return;
    const videoId = channelOverride?.user_login;
    if (!videoId) return;
    // Already installed: the extra deps below cost nothing from here on.
    if (seededYouTubeRef.current.has(providerKey)) return;
    // Timing, and why `isConnected` is in the deps: the Rust side caches this
    // channel's emoji while it resolves chat, so a mount-time call routinely
    // lands BEFORE the data exists and used to fail silently and never retry.
    // `isConnected` cannot go true until that same resolve has run, which makes
    // it an exact "the set is ready now" signal: no polling, no retry timer.
    let cancelled = false;
    void invoke<Array<{ id: string; name: string; url: string; is_global: boolean; locked: boolean }>>(
      'get_youtube_channel_emojis',
      { channel: videoId },
    )
      .then((list) => {
        if (cancelled) return;
        if (!list?.length) {
          if (providerMeta.isConnected) {
            // Connected and still nothing: a signed-out session (YouTube only
            // ships the emoji set to viewers who can type) or a channel with no
            // emoji. Say so once instead of leaving a silently empty tab.
            Logger.debug(`[ChatWidget] No YouTube emoji available for ${videoId}`);
          }
          return;
        }
        seededYouTubeRef.current.add(providerKey);
        useProviderEmoteStore.getState().seed(
          providerKey,
          list.map((e) => ({
            id: e.id,
            name: e.name,
            url: e.url,
            provider: 'youtube' as const,
            // Custom channel emoji lead the grid; YouTube's own set follows.
            emote_type: e.is_global ? 'youtube' : 'custom',
            locked: e.locked,
            lockedLabel: e.locked ? 'Members only' : undefined,
            // Unicode entries insert the literal character, which needs no
            // server-side shortcut lookup. Custom emoji ids are `UC…/hash`, so
            // the slash is what tells the two apart. NOT codepoint length: flags
            // and ZWJ sequences are multi-codepoint and would be misread.
            insertText: e.is_global && !e.id.includes('/') ? e.id : e.name,
          })),
        );
      })
      .catch(() => {
        // Chat still resolving. The isConnected dep brings us back.
      });
    return () => {
      cancelled = true;
    };
  }, [provider, providerKey, channelOverride?.user_login, providerMeta.isConnected, showEmotePicker]);
  const emotes = useMemo(() => {
    if (provider !== 'youtube') return baseEmotes;
    // `baseEmotes` now carries the channel's 7TV set (fetched like Kick's), but
    // it is still null while that fetch is in flight or when the channel has no
    // set at all. Falling back to an empty base rather than returning null is
    // what keeps the picker rendering, since YouTube's own emoji live in
    // `harvestedYouTube` and arrive on a completely separate path.
    return {
      ...(baseEmotes ?? EMPTY_EMOTE_SET),
      youtube: harvestedYouTube ? Array.from(harvestedYouTube.values()) : [],
    };
  }, [baseEmotes, harvestedYouTube, provider]);
  // Forget a channel's harvested emoji when its chat goes away. Bounded per
  // channel already, but without this every YouTube channel opened in a session
  // stays resident for the life of the app.
  useEffect(() => {
    if (provider !== 'youtube' || !providerKey) return;
    return () => {
      useProviderEmoteStore.getState().clear(providerKey);
      // Drop the seeded flag with the data it describes. This pane can switch
      // channels without unmounting, and coming back to a cleared channel has to
      // re-seed rather than trust a stale flag.
      seededYouTubeRef.current.delete(providerKey);
    };
  }, [provider, providerKey]);


  // Shared swapping-smiley state for the emote-picker trigger.
  const smiley = useSwappingSmiley();
  const [isLoadingEmotes, setIsLoadingEmotes] = useState(false);
  const [favoriteEmotes, setFavoriteEmotes] = useState<Emote[]>([]);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const channelPointsRef = useRef<HTMLDivElement>(null);
  // Always-latest reference to handleUsernameClick so window-event listeners
  // (the /usercard and /user slash commands) open the card with current channel
  // context instead of a stale mount-time closure.
  const handleUsernameClickRef = useRef<
    ((userId: string, username: string, displayName: string, color: string, badges: Array<{ key: string; info: any }>, event: React.MouseEvent) => void) | null
  >(null);
  const [isPaused, setIsPaused] = useState(false);
  // "N new since paused" badge. Anchored to the channel's MONOTONIC live-message
  // counter (liveMessageCount) at the instant we enter pause, then shown as a
  // delta — accurate even when the buffer is capped/trimmed, unlike a
  // messages.length diff. See the capture effect below.
  const isHoveringChatRef = useRef<boolean>(false);
  // Ctrl+F routes to the hovered pane, else the main-window chat.
  useEffect(
    () =>
      registerChatSearchController({
        isActive: () => isHoveringChatRef.current === true,
        isMain: () => !channelOverride,
        openSearch: () => setSearchOpen(true),
        closeSearch: () => {
          const was = searchOpenRef.current;
          if (was) setSearchOpen(false);
          return was;
        },
      }),
    [channelOverride],
  );
  const lastResumeTimeRef = useRef<number>(0);
  const lastNavigationTimeRef = useRef<number>(0); // Track scrollToMessage navigation

  // ---- Pause: single source of truth ----------------------------------------
  // `isPaused` (above) is the one authority for paused state. The store's
  // buffer-pause flag is a strict mirror, written ONLY through the single
  // mutator below, so the two can never disagree. Every pause/resume in this
  // component routes through `setChatPaused`:
  //   • `isPausedRef` mirrors the committed value so the guard reads live state
  //     synchronously inside a burst of scroll events. The render-time
  //     `isPaused` closure goes stale mid-burst, which is what let repeated
  //     toggles pile up faster than React could settle a commit.
  //   • `lastPauseToggleRef` rate-limits transitions. Real pause/resume gestures
  //     are hundreds of ms apart, so this settle window is invisible to users
  //     but makes a machine-speed scroll/auto-scroll feedback storm physically
  //     unable to drive an unbounded setState cascade. This is the chat-freeze
  //     guard: at most one transition per window, so the 50-deep render limit
  //     cannot be reached by oscillation. `force` bypasses it for deliberate,
  //     non-repeating transitions (channel switch, message navigation, the
  //     Resume button).
  const isPausedRef = useRef(isPaused);
  const lastPauseToggleRef = useRef(0);
  const setChatPaused = useCallback(
    (next: boolean, opts?: { force?: boolean; scrollToBottom?: boolean }) => {
      if (isPausedRef.current === next) return;
      const now = Date.now();
      if (!opts?.force && now - lastPauseToggleRef.current < PAUSE_SETTLE_MS) return;
      lastPauseToggleRef.current = now;
      isPausedRef.current = next;
      setIsPaused(next);
      setBufferPaused(next);
      if (!next && opts?.scrollToBottom) {
        lastResumeTimeRef.current = now;
        (window as Window & typeof globalThis & { __chatScrollToBottom?: () => void })
          .__chatScrollToBottom?.();
      }
    },
    [setBufferPaused],
  );

  // Dev-only re-render-storm tripwire. ChatWidget sits at the head of the
  // scroll/pause feedback path, so if a future change reintroduces a setState
  // loop, surface it as one labeled error early instead of letting React melt
  // the tree at its 50-deep limit. The settle guard above should keep this from
  // ever firing; it exists to catch the next class of regression. No-op in
  // production. (No dep array: runs once per commit. It never sets state, so it
  // cannot itself cause the loop it watches for.)
  const renderStampsRef = useRef<number[]>([]);
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const stamps = renderStampsRef.current;
    stamps.push(Date.now());
    if (stamps.length > 60) stamps.shift();
    if (stamps.length >= 50 && stamps[stamps.length - 1] - stamps[0] < 1000) {
      Logger.error(
        '[ChatWidget] Re-render storm: 50+ commits in under 1s. A pause/scroll setState loop is likely; the pause settle-guard should bound this, so investigate recent effect or scroll changes.',
      );
      renderStampsRef.current = [];
    }
  });

  const mountTimeRef = useRef<number>(Date.now());
  const [viewerCount, setViewerCount] = useState<number | null>(null);
  const streamUptimeRef = useRef<string>('');
  // Hype train rendering (countdown, level-up, confetti) lives in HypeTrainBanner.
  // We keep the chat container element so the banner can portal its level-up
  // confetti to cover the full chat height.
  const [chatContainerEl, setChatContainerEl] = useState<HTMLElement | null>(null);


  const settings = useAppStore((s) => s.settings);

  // No-input channel-point redemptions (from Twitch's channel-wide community
  // points feed). Message-style and text-input rewards already surface in chat
  // on their own, so we inject only the ones that otherwise wouldn't show, as a
  // native-looking redemption row. Gated by a setting (defaults on).
  // The channel's points icon loads asynchronously and this listener only
  // re-subscribes on channel change, so read the latest value from a ref at
  // redemption time (synced just below the customPointsIconUrl state).
  const customPointsIconRef = useRef<string | null>(null);
  useEffect(() => {
    if (!isTwitch) return;
    const channelId = currentStream?.user_id;
    const channelLogin = currentStream?.user_login;
    if (!channelId || !channelLogin) return;
    const unlisten = listen<{
      channel_id: string;
      user_login: string;
      user_name: string;
      user_id: string;
      reward_id: string;
      reward_title: string;
      reward_cost: number;
      is_input_required: boolean;
      redemption_id: string;
    }>('channel-points-community-redemption', (event) => {
      const p = event.payload;
      if (p.channel_id !== channelId || p.is_input_required) return;
      if (useAppStore.getState().settings.show_channel_point_redemptions === false) return;
      injectRedemptionMessage(channelLogin.toLowerCase(), {
        userLogin: p.user_login,
        userName: p.user_name,
        userId: p.user_id,
        rewardId: p.reward_id,
        rewardTitle: p.reward_title,
        cost: p.reward_cost,
        redemptionId: p.redemption_id,
        pointsIconUrl: customPointsIconRef.current,
      });
    });
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [isTwitch, currentStream?.user_id, currentStream?.user_login]);
  const [selectedUser, setSelectedUser] = useState<{
    userId: string;
    username: string;
    displayName: string;
    color: string;
    badges: Array<{ key: string; info: any }>;
    position: { x: number; y: number };
  } | null>(null);
  const userMessageHistory = useRef<Map<string, ParsedMessage[]>>(new Map());
  // Profile-card history is the only consumer; bound the tracked chatters so a
  // big channel session can't retain parsed messages for thousands of users.
  // Source rooms whose badges were already prefetched this channel session -
  // shared-chat detection runs incrementally per NEW message (it used to
  // rescan the whole buffer every flush).
  const sharedRoomsSeenRef = useRef<Set<string>>(new Set());
  const connectedChannelRef = useRef<string | null>(null);
  // Tracks the room_id we connected with so we can detect a late-arriving
  // broadcaster_id in MultiChat (MultiChatPane resolves channel info async,
  // so the first connect can happen with an empty user_id; badges fail to
  // load until we re-trigger acquireChannel with the real id).
  const connectedRoomIdRef = useRef<string | null>(null);
  // Tracks the channel whose LIVE chat we joined for a VOD's live-toggle, so a
  // replay↔live flip doesn't re-join on every effect run.
  const liveJoinedRef = useRef<string | null>(null);

  // Warm up badge cache on mount (non-blocking, runs before messages render)
  useEffect(() => {
    initializeBadgeImageCache();
  }, []);
  // Track which messages we've already processed (parsed, added to user
  // history, sent to chatUserStore). Previously used an integer high-water
  // mark via `lastProcessedCountRef`, but that silently broke once the
  // messages array rolled past the store cap (slice(N>length) returns empty,
  // skipping all new messages). Using a Set of processed IDs is correct
  // regardless of array rotation and is naturally bounded by `messages.length`
  // because we prune entries no longer present.
  const processedMessageIdsRef = useRef<Set<string>>(new Set());
  const [highlightedMessageId, setHighlightedMessageId] = useState<string | null>(null);
  // Keyboard moderation: the message focused via J/K (a persistent ring, distinct
  // from the transient reply-jump highlight above). Refs mirror the latest values
  // so the controller registered with the keybinding engine reads current data
  // from inside a global keystroke handler.
  const [modFocusId, setModFocusId] = useState<string | null>(null);
  const modFocusIdRef = useRef<string | null>(null);
  const messagesRef = useRef<(string | BackendChatMessage)[]>([]);
  const isModeratorRef = useRef(isModerator);
  const broadcasterIdRef = useRef<string | undefined>(undefined);
  const [isSharedChat, setIsSharedChat] = useState<boolean>(false);
  const [replyingTo, setReplyingTo] = useState<{ messageId: string; username: string } | null>(null);

  // @ mention autocomplete state
  const [showMentionAutocomplete, setShowMentionAutocomplete] = useState(false);
  const [mentionQuery, setMentionQuery] = useState('');
  const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);
  const [mentionStartPosition, setMentionStartPosition] = useState<number | null>(null);
  // These are stable store actions (vanilla zustand never recreates them), so we
  // read them imperatively rather than subscribing. Subscribing here made the
  // whole ChatWidget re-render on every chatUserStore write, and addUser fires
  // once per chatter, so on a busy channel that was a constant re-render storm.
  const { addUser, getMatchingUsers, clearUsers } = useChatUserStore.getState();

  // / command autocomplete state
  const [showCommandAutocomplete, setShowCommandAutocomplete] = useState(false);
  const [commandSelectedIndex, setCommandSelectedIndex] = useState(0);
  const [matchingCommands, setMatchingCommands] = useState<CommandDefinition[]>([]);
  // When the command popup is walking the user through the multi-step /remind
  // flow, this holds the start index of the token Tab/Enter should replace.
  // null means we're completing a top-level command name (the normal path).
  const flowReplaceFromRef = useRef<number | null>(null);

  // Emote tab completion state. The currently-inserted match is matches[index];
  // the carousel shows N back / current / N forward as preview.
  interface EmoteTabState {
    matches: EmoteTabCandidate[];
    index: number;
    /** Cursor position right after the inserted token. */
    expectedCursor: number;
    /** Text the textarea is expected to contain at the moment Tab is processed. */
    expectedValue: string;
    /** Word boundaries of the original (pre-replacement) query. */
    originalStart: number;
    originalQuery: string;
    /** Length of the currently-inserted token (name + optional trailing space). */
    currentLen: number;
  }
  const [emoteTabState, setEmoteTabState] = useState<EmoteTabState | null>(null);

  // Sent-message history for arrow-key recall (Chatterino-style). Newest is at
  // the end. `historyIndex` is -1 when not navigating, otherwise the offset back
  // from the newest entry. `historyDraftRef` preserves whatever was being typed
  // before the user started scrolling back, so ArrowDown can restore it.
  const sentHistoryRef = useRef<string[]>([]);
  const historyDraftRef = useRef<string>('');
  const [historyIndex, setHistoryIndex] = useState(-1);
  const MAX_SENT_HISTORY = 100;

  // Dynamically compute if the current input is a valid, fully-formed command
  const commandState = useMemo(() => {
    if (!messageInput.startsWith('/')) return { isCommand: false, isValid: false, definition: null };
    
    // Check if it's purely a slash
    if (messageInput.trim() === '/') return { isCommand: true, isValid: false, definition: null };

    const parts = messageInput.split(' ');
    const cmdName = parts[0].substring(1).toLowerCase();
    
    const definition = COMMAND_DEFINITIONS.find(c => c.name === cmdName);
    if (!definition) return { isCommand: true, isValid: false, definition: null };
    
    // Count required arguments (those wrapped in <>)
    const usageParts = definition.usage.split(' ').slice(1);
    const requiredArgsCount = usageParts.filter(p => p.startsWith('<') && p.endsWith('>')).length;
    
    // Parse how many arguments the user has provided
    const providedArgsCount = parts.slice(1).filter(p => p.trim() !== '').length;
    
    return {
      isCommand: true,
      isValid: providedArgsCount >= requiredArgsCount,
      definition
    };
  }, [messageInput]);

  // In-field chip overlay for an in-progress /remind command. Active once the
  // command word is solidified (a space follows it); the backdrop below renders
  // the text with each option boxed, aligned over the (transparent-text) textarea.
  const remindOverlay = useMemo(
    () => (/^\/remind\s/i.test(messageInput) ? tokenizeRemindOverlay(messageInput) : null),
    [messageInput],
  );
  const remindOverlayActive = !!remindOverlay;
  const remindBackdropRef = useRef<HTMLDivElement>(null);

  // Spell check for the composer. This replaces the webview's own checker
  // (turned off on the textarea below), which has no idea what an emote is and
  // underlines half of every message.
  //
  // Suppressed while the /remind overlay is up: that mode already swaps the
  // textarea's text for a chipped backdrop at a wider word spacing, so a second
  // mirror at the normal spacing would draw its squiggles in the wrong places.
  const spellcheckEnabled = settings.chat_input?.spellcheck_enabled ?? true;
  const spellEmoteKey = currentStream?.user_login
    ? emoteCacheKey(currentStream.user_login, provider)
    : null;
  const spellUnderlayRef = useRef<HTMLDivElement>(null);
  const spellRanges = useSpellcheck(messageInput, {
    enabled: spellcheckEnabled && !remindOverlayActive,
    emoteKey: spellEmoteKey,
  });
  // Box styling for a solidified /remind token. Horizontal padding is cancelled
  // by an equal negative margin so the chip keeps the plain text's advance width
  // (the caret underneath stays aligned); the gap BETWEEN chips comes from the
  // shared word-spacing applied to both the textarea and this backdrop. Flat
  // filled pill — no border, no bevel.
  const remindChipStyle = (kind: string) => {
    const accent = kind === 'verb' || kind === 'repeat';
    const cmd = kind === 'cmd';
    return {
      borderRadius: '5px',
      padding: '1.5px 0.2em',
      margin: '0 -0.2em',
      background: accent ? 'rgba(200, 224, 232, 0.18)' : cmd ? 'rgba(255, 255, 255, 0.06)' : 'rgba(255, 255, 255, 0.10)',
      color: accent ? 'var(--color-accent)' : cmd ? 'var(--color-text-secondary)' : 'var(--color-text-primary)',
    };
  };

  // Resub notification state
  const [resubNotification, setResubNotification] = useState<ResubNotification | null>(null);
  const [isResubMode, setIsResubMode] = useState(false);
  const [includeStreak, setIncludeStreak] = useState(false);
  const [resubDismissed, setResubDismissed] = useState(false);

  // Watch streak state
  const [watchStreak, setWatchStreak] = useState<WatchStreakMilestone | null>(null);
  const [isWatchStreakMode, setIsWatchStreakMode] = useState(false);
  const [watchStreakDismissed, setWatchStreakDismissed] = useState(false);

  // Drops automation state
  const [dropsCampaign, setDropsCampaign] = useState<{ id: string; name: string; game_name: string } | null>(null);
  const [isDropProgressing, setIsDropProgressing] = useState(false);
  const [isLoadingDrops, setIsLoadingDrops] = useState(false);

  // Channel points state
  const [channelPoints, setChannelPoints] = useState<number | null>(null);
  // Mirror of the balance so the claim callback can compute a true earned
  // amount (balance delta) as a fallback, without depending on render state.
  const channelPointsBalanceRef = useRef<number | null>(null);
  const [channelPointsHovered, setChannelPointsHovered] = useState(false);
  // Bonus-chest claim for the actively watched channel. When auto-claim is on
  // it is collected silently; when off a clickable chest surfaces on the
  // points button. Background automation of channels you are not watching is a
  // separate opt-in plugin, not this.
  const [availableClaim, setAvailableClaim] = useState<{ id: string; channelId: string } | null>(null);
  const [claimingChest, setClaimingChest] = useState(false);
  const claimingChestRef = useRef(false);
  // Brief "+N" feedback on the points button after a chest is collected,
  // whether the user clicked it or auto-claim grabbed it.
  const [claimCelebration, setClaimCelebration] = useState<number | null>(null);
  const claimCelebrationTimer = useRef<number | null>(null);
  const autoClaimWatching = useAppStore((s) => s.settings.auto_claim_points_watching ?? true);
  const [showChannelPointsMenu, setShowChannelPointsMenu] = useState(false);
  const [isLoadingChannelPoints, setIsLoadingChannelPoints] = useState(false);
  const [customPointsName, setCustomPointsName] = useState<string | null>(null);
  const [customPointsIconUrl, setCustomPointsIconUrl] = useState<string | null>(null);
  customPointsIconRef.current = customPointsIconUrl;

  // Pinned chat state
  interface PinnedMessage {
    id: string;
    message_id: string;
    type: string;
    message_text: string;
    sender_id: string;
    sender_name: string;
    sender_color: string;
    sender_avatar: string;
    sender_badges: Array<{ set_id: string; version: string }>;
    pinned_by: string;
    pinned_by_id: string;
    pinned_by_avatar: string;
    started_at: string;
  }
  const [pinnedMessages, setPinnedMessages] = useState<PinnedMessage[]>([]);
  const [isPinnedExpanded, setIsPinnedExpanded] = useState(
    () => !(useAppStore.getState().settings.chat_design?.pinned_start_collapsed ?? true),
  );
  const seenPinIdRef = useRef<string | null>(null);

  // Name -> emote lookup for pinned-message bodies. GQL pins arrive as plain
  // text with no emote ranges, so emotes are matched per whitespace word.
  // Insertion order is reverse priority so the chat's precedence holds:
  // 7TV > BTTV > FFZ > Twitch (later inserts overwrite earlier ones).
  const pinEmoteMap = useMemo(() => {
    const map = new Map<string, Emote>();
    if (!emotes) return map;
    for (const list of [emotes.twitch, emotes.ffz, emotes.bttv, emotes['7tv']]) {
      for (const e of list) map.set(e.name, e);
    }
    return map;
  }, [emotes]);

  // Render a plain-text chunk of a pinned message with emote words swapped for
  // inline images. `emoteClass` sets the image size (expanded body vs collapsed bar).
  const renderPinTextWithEmotes = (text: string, keyPrefix: string, emoteClass: string) => {
    const tokens = text.split(/(\s+)/);
    return tokens.map((tok, i) => {
      const emote = pinEmoteMap.get(tok);
      if (!emote) return <span key={`${keyPrefix}-${i}`}>{tok}</span>;
      return (
        <Tooltip key={`${keyPrefix}-${i}`} content={emote.name}>
          <img
            src={emote.url}
            alt={emote.name}
            className={`inline-block align-middle object-contain ${emoteClass}`}
            loading="lazy"
          />
        </Tooltip>
      );
    });
  };

  // Cache for channel names (broadcaster ID -> display name) used in emote picker grouping
  const [channelNameCache, setChannelNameCache] = useState<Map<string, string>>(new Map());

  // Dynamic chat privileges based on IRC badge context & room state
  const isSubOnly = roomState?.subsOnly || false;
  const isBroadcaster = currentUser?.login && currentStream?.user_login && currentUser.login.toLowerCase() === currentStream.user_login.toLowerCase();
  const canBypassSubOnly = isBroadcaster || (userBadges ? /(broadcaster|moderator|subscriber|founder|vip)/i.test(userBadges) : false);
  // Twitch always sends; Kick sends once a Kick account is connected (OAuth);
  // other providers stay read-only. Disable + label the composer accordingly
  // rather than letting a no-op send swallow input.
  // VOD chat replay is historical — you can't post into the past, so the
  // composer is read-only until the viewer toggles to live chat.
  const isReplayReadOnly = isVodReplay && chatMode === 'replay';
  const canSendHere =
    !isReplayReadOnly &&
    (isTwitch || (provider === 'kick' && kickConnected) || (provider === 'youtube' && youtubeConnected));
  // Sub-only gating is Twitch-only. On a provider we know the room mode but not
  // reliably whether YOU are exempt (the subscription list is an imported
  // snapshot that goes stale), and locking out a real subscriber is worse than
  // letting the platform refuse the message, which is now surfaced with its
  // reason. The mode still shows in the indicator row below.
  const isInputDisabled =
    !canSendHere || !isConnected || (isTwitch && isSubOnly && !canBypassSubOnly);
  // Room modes worth telling the viewer about before they type. Twitch renders
  // none of these today, so this stays provider-only to keep it byte-identical.
  //
  // Only modes that actually GATE YOU are listed. Telling a follower the channel
  // is followers-only describes a restriction they already satisfy, which is
  // noise rather than information — the point of the row is "here is why your
  // message may not send", not an inventory of the channel's settings.
  const providerChannel = channelOverride?.user_login?.toLowerCase();
  const followsThis = useFollowsStore((s) =>
    !isTwitch && providerChannel ? s.isFollowed(provider, providerChannel) : false,
  );
  const subscribesToThis = useFollowsStore((s) =>
    !isTwitch && providerChannel ? s.isSubscribed(provider, providerChannel) : false,
  );
  const providerRoomModes: string[] = [];
  if (!isTwitch && roomState && !isModerator) {
    // Slow and emote-only bind everyone who is not a moderator.
    if (roomState.slow > 0) providerRoomModes.push(`Slow mode · ${roomState.slow}s`);
    if (roomState.emoteOnly) providerRoomModes.push('Emote-only');
    // Kick tells us exactly when we started following, so we can decide whether
    // the requirement actually binds instead of naming it defensively.
    const followedMinutes =
      kickViewer?.following_since != null
        ? (Date.now() - new Date(kickViewer.following_since).getTime()) / 60000
        : followsThis
          ? Number.POSITIVE_INFINITY // following per our own list, age unknown
          : -1;
    const meetsFollowRule =
      followedMinutes >= 0 && followedMinutes >= (roomState.followersOnly ?? 0);
    if ((roomState.followersOnly ?? -1) >= 0 && !meetsFollowRule) {
      providerRoomModes.push(
        roomState.followersOnly > 0
          ? `Followers-only · ${formatFollowAge(roomState.followersOnly)}`
          : 'Followers-only',
      );
    }
    if (roomState.subsOnly && !subscribesToThis) providerRoomModes.push('Subscriber-only');
  }
  const chatPlaceholder = isReplayReadOnly
    ? 'Viewing chat replay (read-only)'
    : !canSendHere
    ? provider === 'kick'
      ? 'Connect your Kick account to send'
      : provider === 'youtube'
      ? 'Connect your YouTube account to send'
      : "Read-only — sending isn't available yet"
    : isWatchStreakMode
    ? "Add a message (optional)..."
    : (isSubOnly && !canBypassSubOnly ? "Subscriber-Only Mode" : "Send a message");

  // Optional composer chrome. The placeholder keeps its text whenever it is
  // saying something the user can act on (read-only, connect your account,
  // sub-only), so hiding it only ever drops the idle "Send a message" prompt.
  const chatInputPrefs = settings.chat_input;
  const showEmoteButton = !chatInputPrefs?.hide_emote_button;
  const showPointsBalance = !chatInputPrefs?.hide_points_balance;
  const visiblePlaceholder =
    chatInputPrefs?.hide_placeholder && !isInputDisabled && !isWatchStreakMode
      ? ''
      : chatPlaceholder;
  // 36px clears the inset emote button; 12px is the plain input inset once it's
  // hidden. The /remind highlight backdrop reuses this so its text stays
  // pixel-aligned with the real textarea underneath it.
  const showCommandButton = !chatInputPrefs?.hide_command_button;
  // Each inset button is 28px wide; the command button sits first, so
  // everything to its right (emote button, send-as picker, text) shifts.
  const commandInset = showCommandButton ? 28 : 0;
  const composerPaddingLeft = `${(showSendAsPicker ? 74 : showEmoteButton ? 36 : 12) + commandInset}px`;

  // Dry run of the /nuke being typed, so you see the blast radius before you
  // commit to it. Deliberately a count in ChatWidget's own state rendered above
  // the input: it never reaches ChatMessageList, so typing can't re-render the
  // message rows. Highlighting the matches in chat would, which is why that is
  // a separate piece of work rather than part of this.
  const [nukePreview, setNukePreview] = useState<
    { messages: number; users: number; action: string } | null
  >(null);

  // Poll / prediction builder. Broadcaster-only, because Twitch scopes both
  // create endpoints to the channel owner.
  const [composerKind, setComposerKind] = useState<'poll' | 'prediction' | null>(null);
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ channel?: string; kind?: string }>).detail;
      // Scoped like the prefill event: only the surface whose channel the
      // command was typed in should open a builder.
      const mine = currentStream?.user_login?.toLowerCase();
      const theirs = detail?.channel?.toLowerCase();
      if (mine && theirs && mine !== theirs) return;
      if (detail?.kind === 'poll' || detail?.kind === 'prediction') setComposerKind(detail.kind);
    };
    window.addEventListener('streamnook-open-poll-composer', handler);
    return () => window.removeEventListener('streamnook-open-poll-composer', handler);
  }, [currentStream?.user_login]);

  useEffect(() => {
    const raw = messageInput.trim();
    const lower = raw.toLowerCase();
    const channel = currentStream?.user_login;
    if (!channel || !isModerator || !lower.startsWith('/nuke ')) {
      setNukePreview(null);
      return;
    }
    const args = raw.slice('/nuke '.length).trim();
    if (!args || args.toLowerCase() === 'stop') {
      setNukePreview(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const mod = await import('../utils/nukeEngine');
        const parsed = mod.parseNukeArgs(args);
        // A half-typed command is the normal case while typing, so an unparseable
        // one just shows nothing rather than an error.
        if ('error' in parsed) {
          if (!cancelled) setNukePreview(null);
          return;
        }
        const { matchedMessages, affected } = mod.previewNuke(channel, parsed);
        if (!cancelled) {
          setNukePreview({
            messages: matchedMessages,
            users: affected.length,
            action: mod.describeAction(parsed.action),
          });
        }
      } catch {
        if (!cancelled) setNukePreview(null);
      }
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [messageInput, currentStream?.user_login, isModerator]);

  // Messages to render
  // Panel wiring: the live key mirrors the acquisition keys used by
  // connectChat; replay mode passes the replay snapshot through since it has
  // no live slice. This is the only parent-side touch of message data, and it
  // changes only on replay ticks / mode flips.
  const panelChannelKey = isTwitch
    ? currentStream?.user_login?.toLowerCase() ?? null
    : providerKey;
  useEffect(() => {
    const key = panelChannelKey;
    const prev = draftKeyRef.current;
    if (prev === key) return;
    if (prev) chatDrafts.set(prev, messageInput);
    draftKeyRef.current = key;
    const restored = key ? chatDrafts.get(key) ?? '' : '';
    // Only touch the input when the channel actually changed.
    setMessageInput(restored);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panelChannelKey]);
  const slowSeconds = roomState?.slow ?? 0;
  const slowRemaining =
    slowSeconds > 0 && lastSentAt > 0 && !isModerator
      ? Math.max(0, Math.ceil((lastSentAt + slowSeconds * 1000 - slowNow) / 1000))
      : 0;
  useEffect(() => {
    if (slowSeconds <= 0 || lastSentAt <= 0 || isModerator) return;
    const until = lastSentAt + slowSeconds * 1000;
    if (Date.now() >= until) return;
    const id = window.setInterval(() => {
      const now = Date.now();
      setSlowNow(now);
      if (now >= until) window.clearInterval(id);
    }, 250);
    return () => window.clearInterval(id);
  }, [slowSeconds, lastSentAt, isModerator]);
  const panelOverride =
    isVodReplay && chatMode === 'replay'
      ? {
          messages: replayChat.messages,
          renderToken: replayChat.renderToken,
          deletedMessageIds: replayChat.deletedMessageIds,
          clearedUserContexts: replayChat.clearedUserContexts,
        }
      : null;
  useEffect(() => {
    setKickModDetected(false);
  }, [panelChannelKey]);



  // Reliably resolve the CURRENT USER's own 7TV cosmetics when chat connects.
  //
  // Twitch never echoes your own PRIVMSG back over IRC, so your own messages
  // exist only as the local optimistic copy — you are the one chatter who never
  // receives a fresh incoming message to re-trigger cosmetics resolution. If your
  // first resolution ever comes back empty (e.g. the App-mount prefetch racing
  // 7TV's warmup, which then caches a stable empty), `chatUserStore` pins your
  // row without a paint for the whole session while everyone else resolves
  // normally during chat. Force a fresh resolution on connect: invalidate any
  // poisoned entry, then re-fetch. The result publishes through cosmeticsCache,
  // so addUser's cache read (on your next send) gets the real paint, and the
  // subscribeToCosmetics bridge repaints an already-rendered own message.
  useEffect(() => {
    // Twitch-only BY MECHANISM, not as a parity gap: this exists purely because
    // Twitch never echoes your own PRIVMSG back, so nothing else ever adds you to
    // the chat-user store. Kick's Pusher room broadcasts your own message like
    // anyone else's, and YouTube's send publishes the rendered item immediately
    // (and the read loop echoes it again), so on both platforms your own message
    // arrives as a normal incoming message and seeds your cosmetics for free.
    // Seeding here for a provider would also key off the TWITCH user id, which is
    // the wrong identity entirely.
    if (!isTwitch) return;
    if (!isConnected) return;
    const selfId = currentUser?.user_id;
    if (!selfId) return;

    // Seed YOUR OWN user into the per-user chat store on connect. Twitch never
    // echoes your own PRIVMSG back over IRC, so the message-processing effect
    // that adds every OTHER chatter to the store never adds YOU. On your first
    // send your row therefore has no store record at all (`hasStoreEntry:false`),
    // so there is nothing to hang your 7TV paint/badge on, AND the
    // cosmetics-repaint bridge bails because it only updates users already in
    // the store. Seeding here guarantees the record exists before your first
    // message renders; forceRefreshCosmetics below then publishes your real
    // cosmetics onto it via the bridge.
    addUser({
      userId: selfId,
      username: currentUser!.login || currentUser!.username || '',
      displayName: currentUser!.display_name || currentUser!.username || selfId,
      color: '#9147FF',
    });

    // Deep refresh: clears BOTH the cosmeticsCache LRU AND the lower-level 7TV
    // userCache before re-fetching. A plain invalidate left a poisoned
    // success-empty (early prefetch racing 7TV warmup) cached in the 7TV layer
    // for 5 minutes, so the "force fresh on connect" still served the empty and
    // your own paint/7TV badge never resolved until something else re-hit 7TV.
    void forceRefreshCosmetics(selfId)
      .then((c) => {
        const sel = (c?.paints || []).find((p: any) => p?.selected);
        // TEMP DIAGNOSTIC [selfpaint]
        Logger.debug('[selfpaint] connect-resolve', {
          selfId,
          paintCount: c?.paints?.length ?? 0,
          selectedPaintId: sel?.id ?? null,
          badgeCount: c?.badges?.length ?? 0,
        });
      })
      .catch((e) => Logger.warn('[selfpaint] connect-resolve failed', e));
  }, [isTwitch, isConnected, currentUser?.user_id]);

  // Viewer count, channel points and pinned messages come from the Rust
  // channel-state service: one watch per channel per window, Rust polls each
  // section on its own cadence (viewers as one Helix batch for every watched
  // channel) and emits only on change. The three JS timers that lived here,
  // and the Helix call the page used to make with credentials handed over by
  // Rust, are gone. Non-Twitch viewer counts still ride the provider override.
  const channelStateLogin = isTwitch ? (currentStream?.user_login?.toLowerCase() ?? null) : null;
  const channelState = useChannelState(channelStateLogin);
  useEffect(() => {
    if (!channelStateLogin || !currentStream?.user_id) return;
    void watchChannel(channelStateLogin, currentStream.user_id);
    return () => {
      void unwatchChannel(channelStateLogin);
    };
  }, [channelStateLogin, currentStream?.user_id]);
  useEffect(() => {
    if (!isTwitch) {
      setViewerCount(currentStream?.viewer_count ?? null);
      return;
    }
    setViewerCount(channelState?.viewer_count ?? null);
  }, [isTwitch, currentStream?.viewer_count, channelState?.viewer_count]);

  // Auto-heal a set whose 7TV rows are not this channel's real dictionary. Rust
  // says so with `seven_tv_ok`: false when the channel document fetch failed and
  // the rows are a fallback (globals only, or a disk copy). Re-fetch on a gentle,
  // visibility-gated cadence so the picker converges on its own once 7TV
  // answers, instead of needing a manual /refresh. Length zero is the legacy
  // signal for sets that predate the flag; a globals-only set is NOT empty,
  // which is why the flag exists.
  useVisibleInterval(() => {
    const login = currentStream?.user_login;
    const id = currentStream?.user_id;
    if (!login || !id) return;
    // Twitch and Kick are the only platforms with a 7TV channel set to wait for.
    // On YouTube/TikTok the set is permanently empty, so this retried forever and
    // each attempt hit Twitch's Helix emote API with a non-Twitch channel id.
    const p = streamProvider(currentStream);
    if (p !== 'twitch' && p !== 'kick') return;
    const set = getChannelEmotes(login, p);
    if (set && (set.seven_tv_ok === false || set['7tv'].length === 0)) {
      void refreshChannelEmotes(login, id, p);
    }
  }, 60000);

  useEffect(() => {
    let headerElement: HTMLElement | null = null;
    const updateUptime = () => {
      // Same clock the Compact View chips use, so the two never drift apart.
      // Empty string when there is no start time, which is what the old inline
      // version wrote too.
      const uptimeString = formatUptimeClock(currentStream?.started_at);
      streamUptimeRef.current = uptimeString;
      if (!headerElement) headerElement = document.getElementById('stream-uptime-display');
      if (headerElement) headerElement.textContent = uptimeString;
    };
    updateUptime();
    const intervalId = setInterval(updateUptime, 1000);
    return () => clearInterval(intervalId);
  }, [currentStream?.started_at]);





  useEffect(() => {
    let timeoutId: ReturnType<typeof setTimeout> | null = null;
    
    // Late-arriving room id: same login, but user_id changed from empty to
    // populated. Re-call connectChat so chatConnectionStore can register the
    // real broadcaster_id (which it needs to populate the Twitch badge cache).
    // This is the MultiChat repaint path — main app sets user_id synchronously
    // before mounting ChatWidget, so this branch is a no-op there.
    if (
      currentStream?.user_login &&
      connectedChannelRef.current === currentStream.user_login &&
      currentStream.user_id &&
      !connectedRoomIdRef.current
    ) {
      connectedRoomIdRef.current = currentStream.user_id;
      // Replay mode never opens the live IRC join (read-only historical chat);
      // still load emotes so replay renders third-party emotes.
      if (!isReplayReadOnly) connectChat(currentStream.user_login, currentStream.user_id);
      loadEmotes(currentStream.user_login, currentStream.user_id);
    }

    if (currentStream?.user_login && connectedChannelRef.current !== currentStream.user_login) {
      connectedChannelRef.current = currentStream.user_login;
      connectedRoomIdRef.current = currentStream.user_id || null;
      // Reset pause state when switching channels - ensures chat starts anchored to bottom
      setChatPaused(false, { force: true });
      mountTimeRef.current = Date.now(); // Reset grace period on channel switch
      // Pass roomId (user_id) to enable fetching recent messages from IVR API.
      // Skipped for a VOD in replay mode: chat is read-only historical, so we
      // bind the channel (for header/emotes) without joining live IRC.
      if (!isReplayReadOnly) connectChat(currentStream.user_login, currentStream.user_id);
      // Defer emote loading until user_id is known. MultiChat pops in with an
      // empty user_id (async stream-info poll), so without this guard we'd
      // fetch globals-only first (Rust caches them under "global"), THEN
      // re-fetch with the real channel id once it arrives. That double-fetch
      // is what makes MultiChat feel sluggish on first open AND briefly shows
      // an empty Twitch tab in the emote picker. The late-arrival branch
      // above handles the deferred fetch once user_id lands.
      if (currentStream.user_id) {
        loadEmotes(currentStream.user_login, currentStream.user_id);
      }
      userMessageHistory.current.clear();
      sharedRoomsSeenRef.current.clear();
      setIsSharedChat(false);
      clearUsers(); // Clear mention autocomplete user list
      // PHASE 3: Clear Rust user message history when switching channels
      invoke('clear_user_message_history').catch(err => 
        Logger.warn('[ChatWidget] Failed to clear Rust user history:', err)
      );
      // Reset channel points when switching channels
      setChannelPoints(null);
      setCustomPointsName(null);
      setCustomPointsIconUrl(null);
      setShowChannelPointsMenu(false);
      // Reset resub notification state when switching channels
      setResubNotification(null);
      setIsResubMode(false);
      setIncludeStreak(false);
      setResubDismissed(false);
      // Reset watch streak state when switching channels
      setWatchStreak(null);
      setIsWatchStreakMode(false);
      setWatchStreakDismissed(false);
      // Reset pinned chat state when switching channels
      setPinnedMessages([]);
      setIsPinnedExpanded(!(useAppStore.getState().settings.chat_design?.pinned_start_collapsed ?? true));
      // Reset to chat view when switching channels
      setActiveView('chat');
      
      // Hot-swap backend tracking context if inside MultiNook.
      //
      // Intentionally skipped in popout mode (`channelOverride` set):
      //   • drops monitoring: popouts are chat-only, not "actively
      //     watching", so we don't collect drops for them.
      //   • EventSub disconnect+reconnect — the EventSub service is single-
      //     broadcaster today. Letting the popout reconnect it would steal
      //     the connection from the main app's stream and break hype train /
      //     raid / pin events there. Multi-broadcaster EventSub is its own
      //     follow-up; until then, popouts piggyback on whatever EventSub
      //     the main app has connected, or get nothing if main isn't
      //     watching this channel.
      //   • register_active_channel — same reason: this marks the "active"
      //     viewing context for backend bookkeeping (drops, automation,
      //     analytics). Popouts aren't an active viewing context.
      if (isMultiNookActive && !channelOverride) {
        Logger.info(`[MultiNook] Hot-swapping backend tracking for ${currentStream.user_login}...`);

        // Immediate clean up front-end state
        useAppStore.getState().setCurrentHypeTrain(null);

        const channelId = currentStream.user_id;
        const channelName = currentStream.user_login;

        if (channelId) {
          // Debounce the backend network shifting (250ms) to prevent UI spamming
          timeoutId = setTimeout(() => {
            invoke('start_drops_monitoring', { channelId, channelName }).catch(e => Logger.warn('[Drops] Failed to hot-swap', e));
            invoke('register_active_channel', { channelId }).catch(() => {});

            invoke('disconnect_eventsub')
              .then(() => {
                // Delay reconnection by 150ms to ensure the OS socket closes safely
                setTimeout(() => {
                  invoke('connect_eventsub', { broadcasterId: channelId }).catch(e => Logger.warn('[EventSub] Failed to hot-swap', e));
                }, 150);
              })
              .catch(() => {});
          }, 250);
        }
      }
    }
    
    // VOD live-chat toggle. The channel is already "bound" (branches above set
    // connectedChannelRef but skipped the IRC join in replay), so a mode flip
    // never re-enters them — join live chat here when the viewer asks for it.
    if (
      isVodReplay &&
      chatMode === 'live' &&
      currentStream?.user_login &&
      liveJoinedRef.current !== currentStream.user_login
    ) {
      liveJoinedRef.current = currentStream.user_login;
      connectChat(currentStream.user_login, currentStream.user_id);
      if (currentStream.user_id) loadEmotes(currentStream.user_login, currentStream.user_id);
    } else if (!(isVodReplay && chatMode === 'live')) {
      liveJoinedRef.current = null;
    }

    return () => {
      if (currentStream?.user_login !== connectedChannelRef.current) connectedChannelRef.current = null;
      if (timeoutId) clearTimeout(timeoutId);
    };
  }, [currentStream?.user_login, currentStream?.user_id, isMultiNookActive, channelOverride, setChatPaused, isVodReplay, chatMode]);

  // Pre-warm the channel's live chat while a VOD is playing, so the FIRST switch
  // to Live shows messages instantly instead of a blank "waiting" screen. The
  // display stays on replay (read-only) until the viewer toggles; this only
  // populates the live slice in the background. Uses `twitchChat.connectChat` —
  // the REAL connect — because the display-selected `connectChat` is the replay
  // no-op while in replay mode. Idempotent per channel; released on unmount.
  useEffect(() => {
    if (!isVodReplay) return;
    const login = currentStream?.user_login;
    if (!login) return;
    void twitchChat.connectChat(login, currentStream?.user_id ?? undefined);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isVodReplay, currentStream?.user_login, currentStream?.user_id]);

  // Force unpause chat when returning from About view
  useEffect(() => {
    if (activeView === 'chat') {
      // Re-trigger the scroll stabilization grace period so the sudden remount
      // doesn't falsely trigger a user scroll-up event
      mountTimeRef.current = Date.now();
      
      // Small delay to let the chat container remount before interacting with it
      const timer = setTimeout(() => {
        setChatPaused(false, { force: true, scrollToBottom: true });
      }, 50);
      return () => clearTimeout(timer);
    }
  }, [activeView, setChatPaused]);

  // Fetch channel points for current channel using direct GQL query with retry logic
  const fetchChannelPoints = useCallback(async () => {
    if (!isTwitch || !channelStateLogin) return;
    await refreshChannelState(channelStateLogin, 'points');
  }, [isTwitch, channelStateLogin]);
  // Points arrive through the channel-state store; mirror them into the
  // local state the header, the tooltip and the claim flow already read. A
  // null / disabled answer clears everything, as the old parser did.
  useEffect(() => {
    if (!isTwitch || channelState?.points_at == null) return;
    const points = channelState.points;
    if (!points || !points.enabled) {
      setChannelPoints(null);
      setCustomPointsName(null);
      setCustomPointsIconUrl(null);
      setAvailableClaim(null);
      setIsLoadingChannelPoints(false);
      return;
    }
    setCustomPointsName(points.name);
    setCustomPointsIconUrl(points.icon_url);
    if (typeof points.balance === 'number') setChannelPoints(points.balance);
    if (points.available_claim_id && currentStream?.user_id) {
      setAvailableClaim({ id: points.available_claim_id, channelId: currentStream.user_id });
    } else {
      setAvailableClaim(null);
    }
    setIsLoadingChannelPoints(false);
  }, [isTwitch, channelState?.points, channelState?.points_at, currentStream?.user_id]);

  // Automatically fetch channel points when entering a new channel. Clear any
  // prior channel's chest first so its claim id can't be clicked against the
  // new channel during the brief fetch window.
  useEffect(() => {
    // The watch above already fetches on channel change; the points effect
    // clears the loading flag when the first answer lands.
    setAvailableClaim(null);
    setIsLoadingChannelPoints(isTwitch && !!currentStream?.user_login);
  }, [isTwitch, currentStream?.user_login]);

  // Keep the balance mirror current for the claim callback's delta fallback.
  useEffect(() => {
    channelPointsBalanceRef.current = channelPoints;
  }, [channelPoints]);

  // Claim the watched channel's bonus chest (manual click or auto). The
  // command returns the exact credited amount (multipliers included) and the
  // new balance; the "+N" pop uses the credited amount.
  const claimWatchedChest = useCallback(async (claimId: string, channelId: string) => {
    if (!isTwitch) return; // channel-points claim is Twitch-only
    if (claimingChestRef.current) return;
    claimingChestRef.current = true;
    setClaimingChest(true);
    try {
      const result = await invoke<{ new_balance: number; points_earned: number }>('claim_channel_points', {
        channelId,
        channelName: currentStream?.user_login ?? '',
        claimId,
      });
      setAvailableClaim(null);
      // Show the true credited amount (multipliers included): prefer the claim
      // response's exact figure, fall back to the balance delta, never a
      // fabricated preset. This matches the points-claimed notification, which
      // reflects the same Twitch-credited amount.
      const prevBalance = channelPointsBalanceRef.current;
      const earned = result.points_earned > 0
        ? result.points_earned
        : (prevBalance !== null && result.new_balance > prevBalance ? result.new_balance - prevBalance : 0);
      if (result.new_balance > 0) setChannelPoints(result.new_balance);
      if (earned > 0) {
        setClaimCelebration(earned);
        if (claimCelebrationTimer.current) window.clearTimeout(claimCelebrationTimer.current);
        claimCelebrationTimer.current = window.setTimeout(() => setClaimCelebration(null), 1800);
        // Keep the profile stat the old backend auto-claim used to feed; the
        // "+N" pop above is the only on-screen feedback (no toast).
        if (currentUser?.user_id) {
          incrementStat(currentUser.user_id, 'channel_points_collected', earned).catch(err => {
            Logger.warn('[ChatWidget] Failed to track channel points stat:', err);
          });
        }
      }
      fetchChannelPoints();
    } catch (err) {
      Logger.warn('[ChatWidget] bonus chest claim failed:', err);
    } finally {
      claimingChestRef.current = false;
      setClaimingChest(false);
    }
  }, [currentStream?.user_login, currentUser?.user_id, fetchChannelPoints]);

  // Instant chest detection: PubSub pushes claim-available the moment Twitch
  // offers the bonus. Detection only — sets availableClaim; the auto-claim
  // effect below decides whether to collect it.
  useEffect(() => {
    if (!currentStream?.user_id) return;
    const channelId = currentStream.user_id;
    const unlisten = listen<{ channel_id?: string | null; claim_id?: string }>(
      'channel-points-claim-available',
      (event) => {
        const claimId = event.payload.claim_id;
        if (!claimId || event.payload.channel_id !== channelId) return;
        setAvailableClaim({ id: claimId, channelId });
      }
    );
    return () => {
      unlisten.then(fn => fn());
    };
  }, [currentStream?.user_id]);

  // (The minute poll for a mid-stream chest now runs in Rust: channel_state
  // polls points every 60 s for every watched channel and emits on change.)

  // Single auto-collect point: every detection path only sets availableClaim;
  // when auto-claim is on, this grabs it. Keeping the claim in one place means
  // detection paths can't double-fire and don't each need claim logic.
  useEffect(() => {
    if (availableClaim && autoClaimWatching && !claimingChestRef.current) {
      claimWatchedChest(availableClaim.id, availableClaim.channelId);
    }
  }, [availableClaim, autoClaimWatching, claimWatchedChest]);

  // Fetch resub notification when entering a new channel
  useEffect(() => {
    if (!isTwitch) return; // resub prompts are Twitch-only
    const fetchResubNotification = async () => {
      if (!currentStream?.user_login || resubDismissed) return;
      
      try {
        const notification = await invoke<ResubNotification | null>('get_resub_notification', {
          channelLogin: currentStream.user_login,
        });
        setResubNotification(notification);
        if (notification) {
          Logger.debug('[ChatWidget] Resub notification available:', notification.cumulative_tenure_months, 'months');
        }
      } catch (err) {
        Logger.warn('[ChatWidget] Failed to fetch resub notification:', err);
        setResubNotification(null);
      }
    };
    
    fetchResubNotification();
  }, [isTwitch, currentStream?.user_login, resubDismissed]);

  // Fetch watch streak milestone when entering a new channel. Skipped in
  // popout mode: popouts are chat-only and shouldn't be
  // surfacing watch-streak prompts (they require actually watching the
  // stream to consume).
  useEffect(() => {
    const fetchWatchStreak = async () => {
      if (channelOverride) return;
      if (!currentStream?.user_id || watchStreakDismissed) return;

      try {
        const milestone = await invoke<WatchStreakMilestone | null>('get_watch_streak', {
          channelId: currentStream.user_id,
        });
        setWatchStreak(milestone);
        if (milestone) {
          Logger.debug('[ChatWidget] Watch streak available:', milestone.streak_count, 'streams, bonus:', milestone.copo_bonus);
        }
      } catch (err) {
        Logger.warn('[ChatWidget] Failed to fetch watch streak:', err);
        setWatchStreak(null);
      }
    };

    fetchWatchStreak();
  }, [currentStream?.user_id, watchStreakDismissed, channelOverride]);

  // Fetch pinned chat messages for current channel. Pins change rarely; the
  // 5s cadence was over-aggressive. 30s + visibility-gating means a tray-
  // backgrounded window stops polling, and a visible window catches pins
  // within half a minute of them being set.
  const fetchPinnedMessages = useCallback(async () => {
    if (!isTwitch || !channelStateLogin) {
      if (isTwitch) setPinnedMessages([]);
      usePinStore.getState().setPinnedIds([]);
      return;
    }
    await refreshChannelState(channelStateLogin, 'pinned');
  }, [isTwitch, channelStateLogin]);
  useEffect(() => {
    if (!isTwitch || channelState?.pinned_at == null) return;
    const messages = (channelState.pinned as PinnedMessage[]) || [];
    setPinnedMessages(messages);
    usePinStore.getState().setPinnedIds(messages.map((m) => m.message_id).filter(Boolean));
  }, [isTwitch, channelState?.pinned, channelState?.pinned_at]);
  // Real-time refresh: any pin/unpin action bumps refreshNonce, so the pin shows
  // up immediately instead of after the 30s poll. A short second pass covers
  // Twitch's brief propagation lag (the Helix 204 can land just before GQL
  // GetPinnedChat reflects it). Skips the initial mount (nonce 0).
  const pinRefreshNonce = usePinStore((s) => s.refreshNonce);
  useEffect(() => {
    if (pinRefreshNonce === 0) return;
    fetchPinnedMessages();
    const t = setTimeout(() => fetchPinnedMessages(), 1200);
    return () => clearTimeout(t);
  }, [pinRefreshNonce, fetchPinnedMessages]);
  // Non-Twitch (e.g. Kick) pins are pushed over the chat bus into the channel
  // slice, not fetched — feed that into the same pinned banner.
  useEffect(() => {
    if (isTwitch) return;
    const pin = providerMeta.pinnedMessage;
    setPinnedMessages(pin ? [pin] : []);
  }, [isTwitch, providerMeta.pinnedMessage]);

  // Listen for channel points updates from backend events. Twitch-only: points
  // do not exist on other platforms, and without this the handlers compared a
  // Twitch channel id against a Kick slug / YouTube UC id on every event.
  useEffect(() => {
    if (!isTwitch || !currentStream?.user_id) return;
    
    const unlistenSpent = listen<{ channel_id?: string | null; points: number; balance: number }>('channel-points-spent', (event) => {
      Logger.debug('[ChatWidget] Points spent event:', event.payload, 'currentChannel:', currentStream.user_id);
      // Update if channel matches OR if no channel_id in event (prediction bets sometimes don't include it)
      if (!event.payload.channel_id || event.payload.channel_id === currentStream.user_id) {
        Logger.debug('[ChatWidget] Updating channel points to:', event.payload.balance);
        setChannelPoints(event.payload.balance);
      }
    });

    const unlistenEarned = listen<{ channel_id?: string | null; points: number; balance: number }>('channel-points-earned', (event) => {
      Logger.debug('[ChatWidget] Points earned event:', event.payload, 'currentChannel:', currentStream.user_id);
      // Update if channel matches OR if no channel_id in event
      if (!event.payload.channel_id || event.payload.channel_id === currentStream.user_id) {
        Logger.debug('[ChatWidget] Updating channel points to:', event.payload.balance);
        setChannelPoints(event.payload.balance);
      }
    });

    return () => {
      unlistenSpent.then(fn => fn());
      unlistenEarned.then(fn => fn());
    };
  }, [currentStream?.user_id]);

  // Load drops data when stream changes to check if game has active drops
  useEffect(() => {
    const loadDropsForStream = async () => {
      if (!currentStream?.game_name) {
        setDropsCampaign(null);
        setIsDropProgressing(false);
        return;
      }
      setIsLoadingDrops(true);
      try {
        // Get active drop campaigns (not inventory - we want all active campaigns)
        const campaigns = await invoke<Array<{ id: string; name: string; game_name: string; game_id: string }>>('get_active_drop_campaigns');
        if (campaigns && campaigns.length > 0) {
          // Find active campaign matching current game
          const gameName = currentStream.game_name.toLowerCase();
          const matchingCampaign = campaigns.find(
            campaign => campaign.game_name?.toLowerCase() === gameName
          );
          if (matchingCampaign) {
            setDropsCampaign(matchingCampaign);
            // Reflect whether an automation plugin is already collecting this game, from
            // the bridge-cached status (check by game_name, not campaign name).
            const dropProgress = useAppStore.getState().liveDropProgress;
            const progressGameName = dropProgress?.current_drop?.game_name?.toLowerCase() ||
              dropProgress?.current_channel?.game_name?.toLowerCase();
            setIsDropProgressing(!!dropProgress?.active && progressGameName === gameName);
          } else {
            setDropsCampaign(null);
            setIsDropProgressing(false);
          }
        } else {
          setDropsCampaign(null);
          setIsDropProgressing(false);
        }
      } catch (err) {
        Logger.warn('[ChatWidget] Could not load drops data:', err);
        setDropsCampaign(null);
      } finally {
        setIsLoadingDrops(false);
      }
    };
    loadDropsForStream();
  }, [currentStream?.game_name]);

  // Listen for automation status changes from anywhere in the app.
  // Real-time updates arrive via the `automation-status-changed` event listener;
  // the polling fallback is just a stale-protection net so we use a 60-min
  // cadence (aligned with the TitleBar backup poll) and visibility-gate it.
  const handleDropProgressChange = useCallback(async () => {
    if (!dropsCampaign || !currentStream?.game_name) return;
    const dropProgress = useAppStore.getState().liveDropProgress;
    const gameName = currentStream.game_name.toLowerCase();
    const progressGameName = dropProgress?.current_drop?.game_name?.toLowerCase() ||
      dropProgress?.current_channel?.game_name?.toLowerCase();
    setIsDropProgressing(!!dropProgress?.active && progressGameName === gameName);
  }, [dropsCampaign, currentStream?.game_name]);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let isMounted = true;

    const setupListener = async () => {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        const unlistenFn = await listen('drop-progress', handleDropProgressChange);
        if (isMounted) {
          unlisten = unlistenFn;
        } else {
          // Unmounted before listen resolved (StrictMode): unlisten now. Guard the
          // promise — Tauri's unlisten can reject during teardown (registry gone).
          void Promise.resolve(unlistenFn()).catch(() => {});
        }
      } catch (err) {
        Logger.warn('[ChatWidget] Failed to set up automation event listener:', err);
      }
    };
    setupListener();

    return () => {
      isMounted = false;
      if (unlisten) unlisten();
    };
  }, [handleDropProgressChange]);

  useVisibleInterval(handleDropProgressChange, 60 * 60 * 1000);

  // Handler to toggle collecting drops for current channel. Automation is plugin-
  // powered; this control only renders when a plugin provides automation, so route
  // start/stop to it.
  const handleToggleAutomation = async () => {
    if (!dropsCampaign) return;
    if (!useAppStore.getState().externalDropsProvider) return;

    if (isDropProgressing) {
      // Stop collecting
      try {
        await invoke('plugins_invoke_action', { action: 'drops.stop', args: {} });
        setIsDropProgressing(false);
        useAppStore.getState().addToast(`Stopped collecting drops for ${dropsCampaign.game_name}`, 'info');
      } catch (err) {
        Logger.error('[ChatWidget] Failed to stop collecting:', err);
        useAppStore.getState().addToast('Failed to stop collecting drops', 'error');
      }
    } else {
      // Start collecting (the plugin resolves an eligible channel itself)
      try {
        await invoke('plugins_invoke_action', { action: 'drops.run', args: { campaign_id: dropsCampaign.id } });
        setIsDropProgressing(true);
        useAppStore.getState().addToast(`Started collecting drops for ${dropsCampaign.game_name}`, 'success');
      } catch (err) {
        Logger.error('[ChatWidget] Failed to start collecting:', err);
        useAppStore.getState().addToast('Failed to start collecting drops', 'error');
      }
    }
  };

  const getMessageId = useCallback((message: string | BackendChatMessage): string | null => {
    if (typeof message !== 'string') {
      return message.id;
    }
    const idMatch = message.match(/(?:^|;)id=([^;]+)/);
    return idMatch ? idMatch[1] : null;
  }, []);

  // /clearmessages — visual-only clear. Snapshots every currently-rendered
  // message id into a hidden set so the renderer skips them. New incoming
  // messages still appear. The set is reset on channel change.
  const [locallyHiddenMessageIds, setLocallyHiddenMessageIds] = useState<Set<string>>(new Set());
  useEffect(() => {
    setLocallyHiddenMessageIds(new Set());
  }, [currentStream?.user_id]);
  useEffect(() => {
    const handler = () => {
      const ids = new Set<string>();
      messagesRef.current.forEach((m) => {
        const id = getMessageId(m);
        if (id) ids.add(id);
      });
      setLocallyHiddenMessageIds(ids);
    };
    window.addEventListener('streamnook-clear-local-chat', handler);
    return () => window.removeEventListener('streamnook-clear-local-chat', handler);
  }, [getMessageId]);

  // Shift-clicking a message's timeout/ban control drops the command into the
  // composer instead of firing it, so the reason can be edited before sending.
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ channel?: string; command?: string }>).detail;
      if (!detail?.command) return;
      // Ignore prefills meant for another surface's channel.
      const mine = currentStream?.user_login?.toLowerCase();
      const theirs = detail.channel?.toLowerCase().replace(/^twitch:/, '');
      if (mine && theirs && mine !== theirs) return;
      setMessageInput(detail.command);
      inputRef.current?.focus({ preventScroll: true });
    };
    window.addEventListener('streamnook-prefill-command', handler);
    return () => window.removeEventListener('streamnook-prefill-command', handler);
  }, [currentStream?.user_login]);

  // /usercard — open the profile card from a slash command via a synthetic
  // mouse-event stub (handleUsernameClick only reads clientX/Y in its fallback
  // path, which rarely fires for users who can already open the app).
  useEffect(() => {
    const handler = (raw: Event) => {
      const e = raw as CustomEvent<{ userId: string; username: string; displayName: string }>;
      if (!e.detail?.userId) return;
      const fakeEvent = {
        clientX: window.innerWidth / 2,
        clientY: window.innerHeight / 2,
      } as unknown as React.MouseEvent;
      // Call through the ref so we always use the latest closure (current
      // channel context), not the one captured when this listener mounted.
      handleUsernameClickRef.current?.(e.detail.userId, e.detail.username, e.detail.displayName, '#9147FF', [], fakeEvent);
    };
    window.addEventListener('streamnook-open-user-card', handler);
    return () => window.removeEventListener('streamnook-open-user-card', handler);
  }, []);

  // Maintain the "N new since paused" badge off the monotonic live-message
  // counter. Anchor the baseline exactly on the not-paused -> paused edge (so a
  // brief pause flicker can't corrupt it), zero it while live, and recompute the
  // delta as messages keep arriving while paused.


  const handleResume = () => {
    // Button-driven resume is deliberate: force past the settle window so it
    // always lands, then glide to the live bottom.
    setChatPaused(false, { scrollToBottom: true, force: true });
  };

  const handleBadgeClick = useCallback(async (badgeKey: string, badgeInfo: any) => {
    const [setId] = badgeKey.split('/');
    // In a popout the badges overlay lives in main, so route there (opening main if
    // Go Live closed it) rather than flipping the popout's own overlay-less store,
    // which renders nothing. In the main window this opens the overlay directly.
    const { openBadgeDetailInMain } = await import('../utils/openBadgesInMain');
    openBadgeDetailInMain(badgeInfo, setId);
  }, []);

  // Helper function to scroll to a specific message by ID
  // Uses container-aware scrolling to avoid scrolling the entire document
  // which can cause the title bar to disappear in Tauri apps with custom decorations
  const scrollToMessage = useCallback((messageId: string, options?: { highlight?: boolean; align?: 'start' | 'center' | 'end' | 'auto' }) => {
    const { highlight = true, align = 'center' } = options || {};

    Logger.debug('[ChatWidget] scrollToMessage called:', { messageId, highlight, align });

    // Find the message index
    const messageIndex = messagesRef.current.findIndex(msg => getMessageId(msg) === messageId);

    if (messageIndex === -1) {
      Logger.warn('[ChatWidget] Message not found in buffer. ID:', messageId);
      return false;
    }

    // Pause chat to prevent auto-scrolling interference. Force: deliberate
    // navigation must win over the settle window.
    setChatPaused(true, { force: true });

    // Mark navigation start time to prevent auto-resume from snapping back to bottom
    lastNavigationTimeRef.current = Date.now();

    // Set highlight immediately if requested
    if (highlight) {
      setHighlightedMessageId(messageId);
    }

    // Use container-aware scrolling instead of scrollIntoView
    // scrollIntoView can cause the entire document to scroll in WebViews,
    // which makes the title bar disappear in Tauri apps with custom decorations
    requestAnimationFrame(() => {
      const element = document.querySelector(`[data-message-id="${messageId}"]`) as HTMLElement | null;
      if (element) {
        // Find the scrollable chat container (ChatMessageList's container)
        const container = element.closest('.overflow-y-auto') as HTMLElement | null;
        if (container) {
          // Calculate the element's position relative to the container
          const elementTopRelative = element.offsetTop;
          const containerScrollTop = container.scrollTop;
          const containerHeight = container.clientHeight;
          const elementHeight = element.offsetHeight;
          
          let targetScrollTop: number;
          
          if (align === 'center') {
            // Center the element in the container
            targetScrollTop = elementTopRelative - (containerHeight / 2) + (elementHeight / 2);
          } else if (align === 'start') {
            // Align element to the top of the container
            targetScrollTop = elementTopRelative;
          } else if (align === 'end') {
            // Align element to the bottom of the container
            targetScrollTop = elementTopRelative - containerHeight + elementHeight;
          } else {
            // 'auto' - scroll minimum distance to make element visible
            const elementTopInView = elementTopRelative - containerScrollTop;
            const elementBottomInView = elementTopInView + elementHeight;
            
            if (elementTopInView < 0) {
              // Element is above viewport, scroll up
              targetScrollTop = elementTopRelative;
            } else if (elementBottomInView > containerHeight) {
              // Element is below viewport, scroll down
              targetScrollTop = elementTopRelative - containerHeight + elementHeight;
            } else {
              // Element is already in view, no scroll needed
              targetScrollTop = containerScrollTop;
            }
          }
          
          // Clamp scroll position to valid range
          targetScrollTop = Math.max(0, Math.min(targetScrollTop, container.scrollHeight - containerHeight));
          
          // Smooth scroll to the target position
          container.scrollTo({
            top: targetScrollTop,
            behavior: 'smooth'
          });
          
          Logger.debug('[ChatWidget] Scrolled to message via container-aware scroll');
        } else {
          Logger.warn('[ChatWidget] Could not find scrollable container for message');
        }
      } else {
        Logger.warn('[ChatWidget] Could not find DOM element for message:', messageId);
      }
    });

    // Clear highlight after animation completes
    if (highlight) {
      setTimeout(() => setHighlightedMessageId(null), 2000);
    }

    return true;
  }, [getMessageId, setChatPaused]);

  const handleReplyClick = useCallback((parentMsgId: string) => {
    Logger.debug('[ChatWidget] handleReplyClick called for parentMsgId:', parentMsgId);

    const success = scrollToMessage(parentMsgId, { highlight: true, align: 'center' });

    if (!success) {
      useAppStore.getState().addToast('Original message is no longer in chat history', 'info');
    }
  }, [scrollToMessage]);

  // ---- Keyboard moderation controller --------------------------------------
  // Bridges the global keybinding engine to this chat. Only the primary chat
  // (no channelOverride) drives keyboard moderation; popout/MultiNook instances
  // opt out so they never clobber the single registered controller.
  const scrollToMessageRef = useRef(scrollToMessage);
  useEffect(() => { scrollToMessageRef.current = scrollToMessage; }, [scrollToMessage]);
  useEffect(() => { isModeratorRef.current = isModerator; }, [isModerator]);
  useEffect(() => { broadcasterIdRef.current = currentStream?.user_id; }, [currentStream?.user_id]);
  useEffect(() => { modFocusIdRef.current = modFocusId; }, [modFocusId]);

  // Drop the focus ring when the channel changes or chat resumes to the live
  // bottom (resuming live reads as "done moderating that spot").
  useEffect(() => { setModFocusId(null); }, [currentStream?.user_id]);
  useEffect(() => { if (!isPaused) setModFocusId(null); }, [isPaused]);

  useEffect(() => {
    // The main app (no override) always owns the moderation keys; in MultiChat the
    // ACTIVE pane owns them, so the popout's hotkeys act on the pane you're looking
    // at rather than every pane registering and clobbering each other.
    const ownsKeys = !channelOverride || channelOverride.is_active === true;
    if (!ownsKeys) return;

    // `id` addresses the DOM row (matches data-message-id); `deleteId` is the
    // IRC `id` tag the Helix delete endpoint expects (the existing per-message
    // delete tool uses the tag, so mirror that and fall back to `id`).
    interface ModMsg { id: string; deleteId: string; userId: string; username: string; displayName: string }
    const list = (): ModMsg[] => {
      const out: ModMsg[] = [];
      for (const m of messagesRef.current) {
        if (typeof m === 'string') continue;
        const bm = m as BackendChatMessage;
        if (!bm.id || !bm.user_id) continue;
        out.push({
          id: bm.id,
          deleteId: (bm.tags && bm.tags['id']) || bm.id,
          userId: bm.user_id,
          username: bm.username,
          displayName: bm.display_name || bm.username,
        });
      }
      return out;
    };
    const focusAt = (items: ModMsg[], idx: number) => {
      const t = items[idx];
      if (!t) return;
      setModFocusId(t.id);
      scrollToMessageRef.current?.(t.id, { highlight: false, align: 'auto' });
    };
    const move = (dir: 1 | -1) => {
      const items = list();
      if (items.length === 0) return;
      const curId = modFocusIdRef.current;
      const curIdx = curId ? items.findIndex((x) => x.id === curId) : -1;
      const idx = curIdx === -1 ? items.length - 1 : Math.max(0, Math.min(items.length - 1, curIdx + dir));
      focusAt(items, idx);
    };
    const focused = (): { items: ModMsg[]; idx: number; t: ModMsg } | null => {
      const curId = modFocusIdRef.current;
      if (!curId) return null;
      const items = list();
      const idx = items.findIndex((x) => x.id === curId);
      if (idx === -1) {
        setModFocusId(null);
        useAppStore.getState().addToast('That message left the chat buffer', 'info');
        return null;
      }
      return { items, idx, t: items[idx] };
    };
    const fmtDur = (s: number) => (s < 60 ? `${s}s` : s < 3600 ? `${s / 60}m` : `${s / 3600}h`);
    const act = async (
      run: (t: ModMsg, broadcasterId: string) => Promise<unknown>,
      label: (t: ModMsg) => string,
      advance: boolean,
    ) => {
      const f = focused();
      if (!f) return;
      const broadcasterId = broadcasterIdRef.current;
      if (!broadcasterId) {
        useAppStore.getState().addToast('No channel to moderate', 'warning');
        return;
      }
      try {
        await run(f.t, broadcasterId);
        useAppStore.getState().addToast(label(f.t), 'success');
        if (advance) {
          const older = f.items[f.idx - 1];
          if (older) focusAt(f.items, f.idx - 1);
          else setModFocusId(null);
        }
      } catch (err) {
        Logger.error('[Mod] keyboard action failed:', err);
        useAppStore.getState().addToast('Moderation action failed', 'error');
      }
    };

    const controller: ChatModController = {
      isModerator: () => isModeratorRef.current,
      hasFocus: () => modFocusIdRef.current !== null,
      focusNewer: () => move(1),
      focusOlder: () => move(-1),
      clearFocus: () => setModFocusId(null),
      openUserCard: () => {
        const f = focused();
        if (!f) return;
        window.dispatchEvent(new CustomEvent('streamnook-open-user-card', {
          detail: { userId: f.t.userId, username: f.t.username, displayName: f.t.displayName },
        }));
      },
      deleteFocused: () => void act(
        (t, b) =>
          isTwitch
            ? invoke('delete_chat_message', { broadcasterId: b, messageId: t.deleteId })
            : provider === 'youtube'
            ? invoke('youtube_delete_message', { channel: youtubeSlug, messageId: t.deleteId })
            : invoke('kick_delete_message', { messageId: t.deleteId }),
        (t) => `Deleted message from ${t.displayName}`,
        true,
      ),
      timeoutFocused: (seconds: number) => void act(
        (t, b) =>
          isTwitch
            ? invoke('ban_user', { broadcasterId: b, targetUserId: t.userId, duration: seconds, reason: null })
            : provider === 'youtube'
            ? invoke('youtube_ban_user', { channel: youtubeSlug, targetChannelId: t.userId, durationSeconds: seconds })
            : invoke('kick_ban_user', {
                broadcasterUserId: Number(b),
                targetUserId: Number(t.userId),
                durationMinutes: kickTimeoutMinutes(seconds),
                reason: null,
              }),
        // Kick rounds to whole minutes, so name the duration it really applies.
        (t) =>
          `Timed out ${t.displayName} (${fmtDur(
            provider === 'kick' ? kickAppliedSeconds(seconds) : seconds,
          )})`,
        true,
      ),
      banFocused: () => void act(
        (t, b) =>
          isTwitch
            ? invoke('ban_user', { broadcasterId: b, targetUserId: t.userId, duration: null, reason: null })
            : provider === 'youtube'
            ? invoke('youtube_ban_user', { channel: youtubeSlug, targetChannelId: t.userId, durationSeconds: null })
            : invoke('kick_ban_user', {
                broadcasterUserId: Number(b),
                targetUserId: Number(t.userId),
                durationMinutes: null,
                reason: null,
              }),
        (t) => `Banned ${t.displayName}`,
        true,
      ),
      unbanFocused: () => void act(
        (t, b) =>
          isTwitch
            ? invoke('unban_user', { broadcasterId: b, targetUserId: t.userId })
            : provider === 'youtube'
            ? invoke('youtube_unban_user', { channel: youtubeSlug, targetChannelId: t.userId })
            : invoke('kick_unban_user', { broadcasterUserId: Number(b), targetUserId: Number(t.userId) }),
        (t) => `Unbanned ${t.displayName}`,
        false,
      ),
    };
    registerChatModController(controller);
    return () => {
      // Singleton: only clear if WE'RE still the registered controller (a different
      // pane may have taken ownership before this cleanup runs).
      if (getChatModController() === controller) registerChatModController(null);
    };
    // All other live data is read through refs, so re-registration only needs
    // to track channelOverride (primary chat vs popout/MultiNook).
  }, [channelOverride]);

  const loadEmotes = async (channelName: string, channelId?: string) => {
    setIsLoadingEmotes(true);
    try {
      // Seed the channel-scoped Twitch badge cache in the background. Channel
      // badges (subscriber, bits, founder) are per-room and are NOT covered by
      // the global set, so without this the room's own badges render blank.
      // Twitch-only: Kick badge art is baked into each message (badges_v2 + sub
      // art), so there's no Twitch badge cache to seed for a Kick channel.
      // Third-party databases need no priming; the Rust side fetches them.
      if (isTwitch && channelId) {
        prefetchChannelBadges(channelId).catch(err =>
          Logger.warn('[ChatWidget] Badge prefetch error (non-blocking):', err),
        );
      }

      // PRIORITY: Fetch emotes first and display immediately.
      // Routes through `ensureChannelEmotes` so the per-channel EmoteSet is
      // shared with any other ChatWidget instances on the same channel (split
      // panes, parallel popouts) instead of each holding its own ~MB-scale
      // copy. The subscription inside `useChannelEmotes` re-renders this
      // component when the cache populates; we still receive the set here
      // for the downstream owner-name + favorite-emote effects.
      const emoteSet = await ensureChannelEmotes(channelName, channelId ?? '', provider);
      setIsLoadingEmotes(false); // Clear loading state immediately after emotes arrive

      // Proactively warm the on-disk cache for the WHOLE set at the size it will
      // render, stream-politely (same single-serial, idle-scheduled queue as
      // display caching — this only feeds it more items, it does not raise the
      // download rate). This is what makes a later open of the picker disk-first
      // instead of re-pulling provider CDNs for every cell on every open.
      if (emoteSet) queueChannelEmotesForCaching(emoteSet);

      // Note: We use loading="lazy" on emote picker images instead of preloading
      // This prevents WebView connection throttling issues with 900+ images

      // BACKGROUND: Fetch channel names for Twitch emote owners (for grouped display)
      // This can happen after emotes are already displayed
      if (emoteSet?.twitch) {
        const ownerIds = new Set<string>();
        for (const emote of emoteSet.twitch) {
          if (emote.owner_id && emote.emote_type === 'subscriptions') {
            ownerIds.add(emote.owner_id);
          }
        }
        
        // Fetch display names for all unique owner IDs (in parallel, non-blocking)
        if (ownerIds.size > 0) {
          const newCache = new Map(channelNameCache);
          const idsToFetch = Array.from(ownerIds).filter(id => !newCache.has(id));
          
          if (idsToFetch.length > 0) {
            Logger.debug(`[ChatWidget] Fetching ${idsToFetch.length} channel names for emote groups`);
            Promise.allSettled(
              idsToFetch.map(async (id) => {
                const user = await invoke<{ display_name: string }>('get_user_by_id', { userId: id });
                return { id, name: user.display_name };
              })
            ).then(results => {
              for (const result of results) {
                if (result.status === 'fulfilled') {
                  newCache.set(result.value.id, result.value.name);
                }
              }
              setChannelNameCache(new Map(newCache));
            });
          }
        }
      }

      // BACKGROUND: Load favorite emotes (non-blocking)
      loadFavoriteEmotes().then(() => {
        if (emoteSet) {
          const allEmotes = [...emoteSet.twitch, ...emoteSet.bttv, ...emoteSet['7tv'], ...emoteSet.ffz, ...emoteSet.kick, ...emoteSet.youtube];
          const availableFavorites = getAvailableFavorites(allEmotes);
          setFavoriteEmotes(availableFavorites);
        }
      }).catch(err => Logger.warn('[ChatWidget] Failed to load favorites:', err));

    } catch (err) {
      Logger.error('Failed to load emotes:', err);
      setIsLoadingEmotes(false);
    }
  };

  // Per-channel last-sent cache for the bypass-duplicate feature. Keyed by
  // currentStream user_id so switching channels resets the tracking.
  const lastSentRef = useRef<Map<string, string>>(new Map());
  // Invisible Plane-14 tag character that Twitch's duplicate-message detector
  // ignores but the IRC pipeline accepts. Used to bypass the rejection when
  // the user opts in. Single char, takes no visual space in chat.
  const DUPLICATE_BYPASS_SUFFIX = ' \u{E0000}';

  // Slash commands on a Kick/YouTube channel. The Twitch handler is Helix, GQL
  // and raw IRC end to end, so nothing here may fall through to it. Moderation
  // verbs map onto the provider's own commands; everything else says so out
  // loud rather than silently doing nothing (or worse, hitting Twitch).
  const handleProviderSlashCommand = async (raw: string) => {
    const notice = (text: string) => {
      if (!providerKey) return;
      injectSystemMessage(providerKey, text, undefined, systemSourceFor(providerKey));
    };

    const parts = raw.slice(1).trim().split(/\s+/);
    const cmd = (parts[0] || '').toLowerCase();
    const args = parts.slice(1);

    // User-defined text macros are platform-neutral, so they keep working here
    // exactly as they do on Twitch (same lookup and expansion as commandHandler).
    const userCommand = findUserCommand(
      cmd,
      useAppStore.getState().settings.chat_commands?.user_commands,
      true,
    );
    if (userCommand) {
      const ctx = buildTemplateContext(
        args,
        channelOverride?.user_id || '',
        channelOverride?.user_login || '',
      );
      const expansion = expandUserCommand(userCommand.expansion, ctx);
      if (expansion.missing_args.length > 0) {
        notice(
          `/${cmd} expects argument${expansion.missing_args.length > 1 ? 's' : ''} ${expansion.missing_args
            .map((i) => `{${i}}`)
            .join(', ')}`,
        );
        return;
      }
      if (expansion.text) await sendMessage(expansion.text);
      return;
    }

    // Resolve a target by name out of the visible buffer: provider chat has no
    // name->id lookup, so a user who hasn't spoken recently can't be targeted.
    const findTarget = (name: string) => {
      const wanted = name.replace(/^@/, '').toLowerCase();
      const buffered = messagesRef.current;
      for (let i = buffered.length - 1; i >= 0; i--) {
        const m = buffered[i];
        if (typeof m === 'string') continue;
        const bm = m as BackendChatMessage;
        if (!bm.user_id) continue;
        if (
          bm.username?.toLowerCase() === wanted ||
          bm.display_name?.toLowerCase() === wanted
        ) {
          return bm;
        }
      }
      return null;
    };

    // This pane's own channel, not the main window's stream: in a MultiChat
    // popout those differ, and moderating the wrong channel is the failure mode
    // this whole pass exists to remove.
    const broadcasterId = channelOverride?.user_id;
    const isYouTube = provider === 'youtube';

    const runModeration = async (
      verb: 'ban' | 'timeout' | 'unban',
      seconds: number | null,
    ) => {
      if (!isModerator) {
        notice(`You don't have moderator permissions in this channel.`);
        return;
      }
      const target = findTarget(args[0] || '');
      if (!target) {
        notice(
          `Can't find "${args[0] || ''}" in recent chat. Moderate from their message instead.`,
        );
        return;
      }
      try {
        if (verb === 'unban') {
          await (isYouTube
            ? invoke('youtube_unban_user', { channel: youtubeSlug, targetChannelId: target.user_id })
            : invoke('kick_unban_user', {
                broadcasterUserId: Number(broadcasterId),
                targetUserId: Number(target.user_id),
              }));
          useAppStore.getState().addToast(`Unbanned ${target.display_name || target.username}`, 'success');
          return;
        }
        const reason = args.slice(verb === 'timeout' ? 2 : 1).join(' ') || null;
        await (isYouTube
          ? invoke('youtube_ban_user', {
              channel: youtubeSlug,
              targetChannelId: target.user_id,
              durationSeconds: seconds,
            })
          : invoke('kick_ban_user', {
              broadcasterUserId: Number(broadcasterId),
              targetUserId: Number(target.user_id),
              durationMinutes: seconds === null ? null : kickTimeoutMinutes(seconds),
              reason,
            }));
        useAppStore
          .getState()
          .addToast(
            verb === 'ban'
              ? `Banned ${target.display_name || target.username}`
              : `Timed out ${target.display_name || target.username}`,
            'success',
          );
      } catch (err) {
        Logger.error('[Provider] moderation command failed:', err);
        useAppStore.getState().addToast('Moderation action failed', 'error');
      }
    };

    switch (cmd) {
      case 'ban':
        await runModeration('ban', null);
        return;
      case 'timeout': {
        const secs = Number(args[1]);
        await runModeration('timeout', Number.isFinite(secs) && secs > 0 ? secs : 600);
        return;
      }
      case 'unban':
      case 'untimeout':
        await runModeration('unban', null);
        return;
      default:
        notice(`/${cmd} isn't available on ${PROVIDERS[provider].label}.`);
    }
  };

  const handleSendMessage = async (opts?: { keepInput?: boolean }) => {
    setLastSentAt(Date.now());
    // Twitch sends need the Twitch account; a provider send needs only that
    // platform's own connection, so a Kick/YouTube user with no Twitch login
    // can still talk.
    const senderReady = isTwitch ? !!currentUser : canSendHere;
    if ((messageInput.trim() || isWatchStreakMode || isResubMode) && isConnected && senderReady) {
      const inputSettings = useAppStore.getState().settings.chat_input;
      const keepInput = !!opts?.keepInput;
      let messageToSend = messageInput;

      // Bypass duplicate — if this message is identical to the last one we
      // sent on this channel, append the invisible suffix so Twitch accepts
      // the second send. Skipped for slash commands (which Twitch doesn't
      // dedupe) and resub/streak modes (different code path entirely).
      const channelKey = currentStream?.user_id || '_';
      if (
        isTwitch &&
        inputSettings?.bypass_duplicate &&
        !messageToSend.startsWith('/') &&
        !isResubMode &&
        !isWatchStreakMode &&
        lastSentRef.current.get(channelKey) === messageToSend
      ) {
        messageToSend = messageToSend + DUPLICATE_BYPASS_SUFFIX;
      }
      // Record what we sent (the original, pre-suffix form) so a third send
      // of the same text also triggers the bypass.
      lastSentRef.current.set(channelKey, messageInput);

      // Push the sent text onto the arrow-key recall history. De-dupe against
      // the immediately-previous entry and cap the buffer. Reset the navigation
      // cursor so the next ArrowUp starts from this newest message.
      const trimmedForHistory = messageInput.trim();
      if (trimmedForHistory) {
        const hist = sentHistoryRef.current;
        if (hist[hist.length - 1] !== messageInput) {
          hist.push(messageInput);
          if (hist.length > MAX_SENT_HISTORY) hist.shift();
        }
      }
      setHistoryIndex(-1);

      const replyParentMsgId = replyingTo?.messageId;
      if (!keepInput) {
        setMessageInput('');
        setReplyingTo(null);
        // Reset textarea height after sending
        if (inputRef.current) {
          inputRef.current.style.height = '36px';
        }
      }
      inputRef.current?.focus({ preventScroll: true });

      // Handle resub mode - send resub notification
      if (isResubMode && resubNotification && currentStream?.user_login) {
        setIsResubMode(false);
        try {
          const success = await invoke<boolean>('use_resub_token', {
            channelLogin: currentStream.user_login,
            message: messageToSend || null,
            includeStreak: includeStreak,
            tokenId: resubNotification.id,
          });
          if (success) {
            setResubNotification(null); // Token consumed
            setResubDismissed(true);
          } else {
            useAppStore.getState().addToast('Failed to share resub notification', 'error');
            setMessageInput(messageToSend);
          }
        } catch (err) {
          Logger.error('[ChatWidget] Failed to use resub token:', err);
          useAppStore.getState().addToast('Failed to share resub notification', 'error');
          setMessageInput(messageToSend);
        }
        return;
      }

      // Handle watch streak mode - send watch streak share
      if (isWatchStreakMode && watchStreak && currentStream?.user_id) {
        setIsWatchStreakMode(false);
        try {
          const success = await invoke<boolean>('share_watch_streak', {
            channelId: currentStream.user_id,
            milestoneId: watchStreak.milestone_id,
            message: messageToSend || null,
          });
          if (success) {
            setWatchStreak(null); // Milestone consumed
            setWatchStreakDismissed(true);
            useAppStore.getState().addToast('Watch streak shared!', 'success');
          } else {
            useAppStore.getState().addToast('Failed to share watch streak', 'error');
            setMessageInput(messageToSend);
          }
        } catch (err) {
          Logger.error('[ChatWidget] Failed to share watch streak:', err);
          useAppStore.getState().addToast('Failed to share watch streak', 'error');
          setMessageInput(messageToSend);
        }
        return;
      }

      try {
        // Plain-text user commands (require_slash: false). Run BEFORE the
        // slash-command intercept so a non-slash trigger expansion can still
        // start with "/" — the expansion result is re-evaluated as a new
        // message body, including being able to fire a built-in slash command.
        if (!messageToSend.startsWith('/')) {
          const plainMatch = matchPlainTextUserCommand(
            messageToSend,
            useAppStore.getState().settings.chat_commands?.user_commands,
          );
          if (plainMatch) {
            const ctx = buildTemplateContext(
              plainMatch.args,
              currentStream?.user_id || '',
              currentStream?.user_login || '',
            );
            const expansion = expandUserCommand(plainMatch.command.expansion, ctx);
            if (expansion.missing_args.length > 0) {
              useAppStore.getState().addToast(
                `Trigger "${plainMatch.command.trigger}" expects argument${expansion.missing_args.length > 1 ? 's' : ''} ${expansion.missing_args.map((i) => `{${i}}`).join(', ')}`,
                'error',
              );
              return;
            }
            if (!expansion.text) {
              useAppStore.getState().addToast(`Trigger "${plainMatch.command.trigger}" expanded to an empty message`, 'error');
              return;
            }
            // Re-enter with the expanded text. Replace input and recurse via
            // setMessageInput so the existing flow handles the slash-command
            // detection, optimistic send, etc.
            setMessageInput(expansion.text);
            // Defer one tick so the input state lands before the next send.
            setTimeout(() => {
              const form = inputRef.current?.form;
              form?.requestSubmit();
            }, 0);
            return;
          }
        }

        // Provider slash commands never reach the Twitch handler below: every
        // command in it is Helix/GQL or raw IRC, so a Kick/YouTube channel id
        // would be sent to Twitch's API for a completely unrelated channel.
        if (messageToSend.startsWith('/') && !isTwitch) {
          await handleProviderSlashCommand(messageToSend);
          return;
        }

        // Intercept slash commands
        if (messageToSend.startsWith('/')) {
          const handled = await handleSlashCommand(
            messageToSend, 
            currentStream?.user_id || '', 
            currentStream?.user_login || '',
            (msg) => sendMessage(msg, {
              username: currentUser!.login || currentUser!.username,
              displayName: currentUser!.display_name || currentUser!.username,
              userId: currentUser!.user_id,
              color: undefined,
              badges: ''
            })
          );
          
          if (handled) {
            return;
          }

          // Unhandled slash commands (like /mods, /vips, /me) get sent directly
          // through IRC without optimistic rendering. Twitch IRC responds with
          // a NOTICE message which the frontend already handles inline.
          await invoke('send_chat_message', {
            message: messageToSend,
            replyParentMsgId: null,
          });
          return;
        }

        // Badges come from the IRC USERSTATE cached inside useTwitchChat
        // (populated on channel JOIN). The previous per-send Helix round-trip
        // returned a strictly inferior subset (no tenure, no mod/VIP/etc), so
        // it's intentionally gone.
        // If a linked secondary account is selected in the picker, send as it.
        const sendState = useSendAccountStore.getState();
        const chosen = sendState.sendAsId
          ? sendState.accounts.find((a) => a.user_id === sendState.sendAsId)
          : undefined;
        const senderAccount =
          chosen && !chosen.is_primary
            ? { userId: chosen.user_id, login: chosen.login, displayName: chosen.display_name }
            : undefined;

        await sendMessage(messageToSend, currentUser ? {
          username: currentUser.login || currentUser.username,
          displayName: currentUser.display_name || currentUser.username,
          userId: currentUser.user_id,
          color: undefined,
          badges: ''
        } : undefined, replyParentMsgId, senderAccount);

        // Stats and emote tallies are keyed by the Twitch user id, so they only
        // apply when a Twitch account is linked.
        if (currentUser) {
          // Track message sent stat for analytics
          incrementStat(currentUser.user_id, 'messages_sent', 1).catch(err => {
            Logger.warn('[ChatWidget] Failed to track message sent stat:', err);
          });

          // Tally emote usage from this message into the member's persisted
          // most-used-emotes counts (best effort, non-blocking).
          void trackEmoteUsage(messageToSend, currentStream?.user_id || null, currentUser.user_id);
        }
      } catch (err) {
        Logger.error('Failed to send message:', err);
        setMessageInput(messageToSend);
        // A provider refusal carries the platform's own reason (timeout, slow
        // mode, banned word); showing it beats a generic retry prompt.
        const reason =
          !isTwitch && err instanceof Error && err.message
            ? err.message
            : 'Failed to send message. Please try again.';
        useAppStore.getState().addToast(reason, 'error');
      }
    }
  };

  const handleKeyPress = (e: React.KeyboardEvent) => {
    // Handle autocomplete navigation when visible
    if (showMentionAutocomplete) {
      const matchingUsers = getMatchingUsers(mentionQuery);
      
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setMentionSelectedIndex(prev => 
          prev > 0 ? prev - 1 : matchingUsers.length - 1
        );
        return;
      }
      
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setMentionSelectedIndex(prev => 
          prev < matchingUsers.length - 1 ? prev + 1 : 0
        );
        return;
      }
      
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        const selectedUser = matchingUsers[mentionSelectedIndex];
        if (selectedUser) {
          insertMention(selectedUser);
        }
        return;
      }
      
      if (e.key === 'Escape') {
        e.preventDefault();
        setShowMentionAutocomplete(false);
        return;
      }
      
      if (e.key === 'Tab') {
        e.preventDefault();
        const selectedUser = matchingUsers[mentionSelectedIndex];
        if (selectedUser) {
          insertMention(selectedUser);
        }
        return;
      }
    }
    
    // Handle command autocomplete navigation when visible
    if (showCommandAutocomplete) {
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setCommandSelectedIndex(prev => 
          prev > 0 ? prev - 1 : matchingCommands.length - 1
        );
        return;
      }
      
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setCommandSelectedIndex(prev => 
          prev < matchingCommands.length - 1 ? prev + 1 : 0
        );
        return;
      }
      
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        const selectedCmd = matchingCommands[commandSelectedIndex];
        if (selectedCmd) {
          insertCommand(selectedCmd);
        }
        return;
      }
      
      if (e.key === 'Escape') {
        e.preventDefault();
        setShowCommandAutocomplete(false);
        return;
      }
      
      if (e.key === 'Tab') {
        e.preventDefault();
        const selectedCmd = matchingCommands[commandSelectedIndex];
        if (selectedCmd) {
          insertCommand(selectedCmd);
        }
        return;
      }
    }
    
    // Arrow-key recall of previously-sent messages (Chatterino-style). Active
    // only when neither autocomplete dropdown is open (those return above for
    // arrows). ArrowUp walks back through history when the caret is on the first
    // line; ArrowDown walks forward and finally restores the in-progress draft.
    if (!showMentionAutocomplete && !showCommandAutocomplete && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      const ta = inputRef.current;
      const history = sentHistoryRef.current;
      if (ta && history.length > 0) {
        const noSelection = ta.selectionStart === ta.selectionEnd;
        const onFirstLine = ta.value.slice(0, ta.selectionStart).indexOf('\n') === -1;
        const onLastLine = ta.value.slice(ta.selectionStart).indexOf('\n') === -1;

        const applyRecalled = (text: string) => {
          setMessageInput(text);
          // Caret to the end after React commits the new value.
          requestAnimationFrame(() => {
            const el = inputRef.current;
            if (el) {
              const end = el.value.length;
              el.setSelectionRange(end, end);
            }
          });
        };

        if (e.key === 'ArrowUp' && noSelection && onFirstLine) {
          e.preventDefault();
          if (historyIndex === -1) historyDraftRef.current = messageInput;
          const nextIndex = Math.min(historyIndex + 1, history.length - 1);
          setHistoryIndex(nextIndex);
          applyRecalled(history[history.length - 1 - nextIndex]);
          return;
        }

        if (e.key === 'ArrowDown' && noSelection && onLastLine && historyIndex !== -1) {
          e.preventDefault();
          const nextIndex = historyIndex - 1;
          if (nextIndex < 0) {
            setHistoryIndex(-1);
            applyRecalled(historyDraftRef.current);
          } else {
            setHistoryIndex(nextIndex);
            applyRecalled(history[history.length - 1 - nextIndex]);
          }
          return;
        }
      }
    }

    // Emote tab completion (only when no other autocomplete is active).
    if (e.key === 'Tab') {
      const used = handleEmoteTabPress(e.shiftKey);
      if (used) {
        e.preventDefault();
        return;
      }
    }

    // Any other key beyond Tab/Shift drops the tab-cycle state. We can't gate
    // this inside handleInputChange because non-printing keys (arrows, Home)
    // also move the cursor away from the expected position.
    if (
      e.key !== 'Tab' &&
      e.key !== 'Shift' &&
      e.key !== 'Control' &&
      e.key !== 'Alt' &&
      e.key !== 'Meta' &&
      emoteTabState
    ) {
      setEmoteTabState(null);
    }

    // Normal Enter to send message. Ctrl+Enter is "Quick Send" — sends but
    // keeps the message in the input box for rapid-fire re-send. Shift+Enter
    // still inserts a newline (textarea default behavior).
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      const quickSendEnabled = useAppStore.getState().settings.chat_input?.quick_send ?? false;
      const keepInput = quickSendEnabled && (e.ctrlKey || e.metaKey);
      handleSendMessage({ keepInput });
    }
  };

  // Auto-resize textarea whenever messageInput changes
  useEffect(() => {
    const textarea = inputRef.current;
    if (!textarea) return;
    
    // Temporarily set overflow hidden to get accurate scrollHeight
    textarea.style.overflow = 'hidden';
    textarea.style.height = 'auto'; // Reset to calculate new height
    const maxHeight = 120; // ~5 lines max
    const newHeight = Math.min(textarea.scrollHeight, maxHeight);
    textarea.style.height = `${newHeight}px`;
    // Enable scrolling only if we hit max height
    textarea.style.overflow = newHeight >= maxHeight ? 'auto' : 'hidden';
  }, [messageInput]);

  // Handle input changes and detect @ mentions
  // Paste an image -> upload through Rust to the configured host -> insert the
  // link. Only when the user enabled the uploader; otherwise paste is untouched.
  const handlePaste = useCallback(
    async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
      const up = useAppStore.getState().settings.chat_input?.image_uploader;
      if (!up?.enabled) return;
      const file = Array.from(e.clipboardData?.files ?? []).find((f) => f.type.startsWith('image/'));
      if (!file) return;
      e.preventDefault();
      const addToast = useAppStore.getState().addToast;
      const target = resolveUploadTarget(up);
      addToast(`Uploading image to ${target.label}…`, 'info');
      try {
        const buf = new Uint8Array(await file.arrayBuffer());
        const link = await invoke<string>('upload_image', buf, {
          headers: {
            'x-upload-url': target.url,
            'x-form-field': target.formField,
            'x-extra-fields': encodeExtraFields(target.extraFields),
            'x-response-path': target.responsePath,
            'x-filename': file.name || 'image.png',
            'x-mime': file.type || 'image/png',
          },
        });
        setMessageInput((prev) => (prev && !prev.endsWith(' ') ? `${prev} ${link} ` : `${prev}${link} `));
        addToast('Image link inserted', 'success');
      } catch (err) {
        addToast(typeof err === 'string' ? err : 'Image upload failed', 'error');
      }
    },
    [],
  );

  const handleInputChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value;
    const cursorPos = e.target.selectionStart || value.length;

    // User typed/edited the input (controlled setMessageInput from Tab doesn't
    // fire this event), so any Tab-cycle in progress no longer matches.
    if (emoteTabState) setEmoteTabState(null);
    // A manual edit ends arrow-key history navigation; next ArrowUp restarts
    // from the newest entry (React no-ops when already -1).
    setHistoryIndex(-1);

    setMessageInput(value);
    
    // Check for Command Autocomplete (slash commands)
    const isCommand = value.startsWith('/');
    if (isCommand) {
      // Find space to determine if we are still typing the command or the arguments
      const firstSpaceIndex = value.indexOf(' ');
      
      if (firstSpaceIndex === -1 && value.length > 0) {
        // Still typing the command itself
        const query = value.slice(1).toLowerCase(); // remove '/'
        
        // Filter commands based on roles, then append the user's own custom
        // commands. User commands are always available regardless of role.
        const availableCommands = COMMAND_DEFINITIONS.filter(cmd => {
          if (cmd.category === 'Everyone') return true;
          if (cmd.category === 'Moderator' || cmd.category === 'Chat Flow') return isModerator;
          if (cmd.category === 'Engagement' || cmd.category === 'Broadcaster') return isModerator || isBroadcaster;
          return false;
        });
        const userCommands = buildUserCommandDefinitions(settings.chat_commands?.user_commands);
        const allCommands = [...availableCommands, ...userCommands];

        const matches = allCommands.filter(cmd => cmd.name.toLowerCase().startsWith(query));
        setMatchingCommands(matches);
        flowReplaceFromRef.current = null; // completing a command name, not a /remind arg
        setShowCommandAutocomplete(matches.length > 0);
        setCommandSelectedIndex(0);

        // don't show mention autocomplete
        setShowMentionAutocomplete(false);
        return;
      } else {
        // We've typed past the command name (there's a space).
        const cmdName = value.substring(1, firstSpaceIndex).toLowerCase();

        // Guided /remind flow: keep the command popup open and walk the user
        // through each step (subcommand → when → message, or which reminder to
        // manage). Each accepted step stays in the input and the next appears.
        if (cmdName === 'remind') {
          const flow = getRemindFlowSuggestions(value, settings.reminders?.reminders ?? []);
          if (flow && flow.suggestions.length > 0) {
            setMatchingCommands(flow.suggestions);
            flowReplaceFromRef.current = flow.replaceFrom;
            setShowCommandAutocomplete(true);
            setCommandSelectedIndex(0);
            setShowMentionAutocomplete(false);
            return;
          }
          // No suggestions for this step (e.g. typing the message body): drop the
          // popup and let the normal mention/emote autocomplete run below.
          setShowCommandAutocomplete(false);
          flowReplaceFromRef.current = null;
        } else {
          // Other commands: hide the command popup.
          setShowCommandAutocomplete(false);
          flowReplaceFromRef.current = null;
        }

        // Detect if we are typing a command parameter that expects a username
        const cmdDef = COMMAND_DEFINITIONS.find(c => c.name.toLowerCase() === cmdName);

        if (cmdDef && (cmdDef.usage.includes('<username>') || cmdDef.usage.includes('[username]'))) {
          const afterCommand = value.substring(firstSpaceIndex + 1);
          const secondSpaceIndex = afterCommand.indexOf(' ');
          
          if (secondSpaceIndex === -1 && cursorPos > firstSpaceIndex) {
            const query = afterCommand;
            setMentionQuery(query);
            setMentionStartPosition(firstSpaceIndex);
            setMentionSelectedIndex(0);
            
            const matches = getMatchingUsers(query);
            setShowMentionAutocomplete(matches.length > 0);
            return;
          }
        }
      }
    } else {
      setShowCommandAutocomplete(false);
    }

    // Find the @ trigger before cursor
    const textBeforeCursor = value.slice(0, cursorPos);
    const lastAtIndex = textBeforeCursor.lastIndexOf('@');
    
    if (lastAtIndex !== -1) {
      // Check if there's a space between @ and cursor (if so, not a mention)
      const textAfterAt = textBeforeCursor.slice(lastAtIndex + 1);
      const hasSpace = textAfterAt.includes(' ');
      
      // Also check if @ is at start or preceded by whitespace
      const charBeforeAt = lastAtIndex > 0 ? textBeforeCursor[lastAtIndex - 1] : ' ';
      const isValidStart = /\s/.test(charBeforeAt) || lastAtIndex === 0;
      
      if (!hasSpace && isValidStart) {
        const query = textAfterAt;
        setMentionQuery(query);
        setMentionStartPosition(lastAtIndex);
        setMentionSelectedIndex(0);
        
        // Only show autocomplete if we have matches
        const matches = getMatchingUsers(query);
        setShowMentionAutocomplete(matches.length > 0);
        return;
      }
    }
    
    // No valid @ trigger found
    setShowMentionAutocomplete(false);
    setMentionQuery('');
    setMentionStartPosition(null);
  }, [getMatchingUsers, isModerator, isBroadcaster, settings.chat_commands?.user_commands, settings.reminders?.reminders, emoteTabState]);

  // Insert a command string directly from other UI elements (like UserProfileCard)
  const preFillCommand = useCallback((cmdText: string) => {
    setMessageInput(cmdText);
    
    // Hide autocomplete just in case
    setShowCommandAutocomplete(false);
    
    // Focus and set cursor position at end
    inputRef.current?.focus({ preventScroll: true });
    setTimeout(() => {
      inputRef.current?.setSelectionRange(cmdText.length, cmdText.length);
    }, 0);
  }, []);

  // Insert a command at the current slash position
  // Roll the input into the /remind flow's next step (or close the popup when
  // there's nothing left to suggest). Shared by the two insert paths below.
  const advanceRemindFlow = useCallback((newValue: string) => {
    const next = getRemindFlowSuggestions(newValue, useAppStore.getState().settings.reminders?.reminders ?? []);
    if (next && next.suggestions.length > 0) {
      setMatchingCommands(next.suggestions);
      flowReplaceFromRef.current = next.replaceFrom;
      setShowCommandAutocomplete(true);
      setCommandSelectedIndex(0);
    } else {
      setShowCommandAutocomplete(false);
      flowReplaceFromRef.current = null;
    }
  }, []);

  // Everything the command menu can show: every built-in plus the user's own
  // commands. Unlike the slash autocomplete it does not filter by role; the
  // menu dims and locks what this channel does not allow, so the whole
  // command set stays discoverable.
  const browsableCommands = useMemo(
    () => [...COMMAND_DEFINITIONS, ...buildUserCommandDefinitions(settings.chat_commands?.user_commands)],
    [settings.chat_commands?.user_commands],
  );

  const insertCommand = useCallback((cmd: CommandDefinition) => {
    const replaceFrom = flowReplaceFromRef.current;

    // Guided /remind flow: replace the current token and advance to the next
    // step. A hint row has nothing to insert — keep focus and let the user type.
    if (replaceFrom !== null) {
      if (cmd.hint) {
        inputRef.current?.focus({ preventScroll: true });
        return;
      }
      const newValue = `${messageInput.slice(0, replaceFrom)}${cmd.insertText ?? cmd.name} `;
      setMessageInput(newValue);
      advanceRemindFlow(newValue);
      inputRef.current?.focus({ preventScroll: true });
      setTimeout(() => {
        inputRef.current?.setSelectionRange(newValue.length, newValue.length);
      }, 0);
      return;
    }

    // Normal path: complete a top-level command name with a trailing space.
    const newValue = `/${cmd.name} `;
    setMessageInput(newValue);

    // For /remind, step straight into its guided flow instead of closing.
    if (cmd.name.toLowerCase() === 'remind') {
      advanceRemindFlow(newValue);
    } else {
      setShowCommandAutocomplete(false);
      flowReplaceFromRef.current = null;
    }

    inputRef.current?.focus({ preventScroll: true });
    setTimeout(() => {
      inputRef.current?.setSelectionRange(newValue.length, newValue.length);
    }, 0);
  }, [messageInput, advanceRemindFlow]);

  // Insert a mention at the current trigger position
  const insertMention = useCallback((user: { username: string; displayName: string }) => {
    if (mentionStartPosition === null) return;
    
    const triggerChar = messageInput.charAt(mentionStartPosition);
    const isCommandArg = triggerChar === ' ';
    
    const beforeMention = messageInput.slice(0, mentionStartPosition);
    const afterMention = messageInput.slice(mentionStartPosition + 1 + mentionQuery.length);
    
    // Commands expect raw usernames, whereas normal mentions need the @ prefix
    const prefix = isCommandArg ? ' ' : '@';
    // A YouTube "username" is a lowercased display name that can contain spaces,
    // so mentioning one needs the display name as YouTube itself writes it.
    // Twitch and Kick logins are the canonical mention token on their platforms.
    const mentionText =
      provider === 'youtube' ? user.displayName || user.username : user.username;
    const newValue = `${beforeMention}${prefix}${mentionText} ${afterMention}`;
    setMessageInput(newValue);

    // Hide autocomplete
    setShowMentionAutocomplete(false);
    setMentionQuery('');
    setMentionStartPosition(null);

    // Focus and set cursor position after the inserted mention
    inputRef.current?.focus({ preventScroll: true });
    const newCursorPos = beforeMention.length + mentionText.length + 2; // +2 for @ and space
    setTimeout(() => {
      inputRef.current?.setSelectionRange(newCursorPos, newCursorPos);
    }, 0);
  }, [messageInput, mentionStartPosition, mentionQuery, provider]);

  const insertEmote = (emoteName: string) => {
    setMessageInput(prev => prev + (prev ? ' ' : '') + emoteName + ' ');
    inputRef.current?.focus({ preventScroll: true });
  };

  /**
   * Build a ranked, deduplicated list of tab-completion candidates for the
   * partial word the user is typing:
   *   - All loaded emote sets feed the matcher.
   *   - Optionally chatter display names are appended at the lowest tier.
   *   - Match mode is prefix-only ("starts_with") or substring ("includes").
   *   - Dedupe by case-folded name so cross-provider duplicates collapse.
   *   - A leading ':' (':Pog') is stripped before matching and flips the
   *     provider order to Twitch-first, so Twitch-native / sub emotes lead.
   *     A leading '@' instead prefixes the chatter match list.
   *
   * Ranking is a strict (providerTier, favoriteRank, alphabetical) tuple so a
   * favorited Twitch emote never jumps over a non-favorited 7TV match. Default
   * provider tiers, lowest = best: 7tv (0), bttv (1), ffz (2), twitch (3),
   * chatter (4); a colon query reorders providers to twitch (0), 7tv (1),
   * bttv (2), ffz (3) and drops chatters. Within a provider, favorited emotes
   * come first, then alphabetical.
   */
  const TAB_MATCH_LIMIT = 50;
  const getMatchingEmoteTokens = useCallback((query: string): EmoteTabCandidate[] => {
    if (!query) return [];
    const mode: 'starts_with' | 'includes' = settings.chat_input?.emote_tab_complete_match_mode ?? 'starts_with';
    const includeChatters = settings.chat_input?.emote_tab_complete_include_chatters ?? true;
    const q = query.toLowerCase();
    const seen = new Set<string>();
    type Ranked = { item: EmoteTabCandidate; providerTier: number; favoriteRank: number };
    const ranked: Ranked[] = [];

    const isAtQuery = q.startsWith('@');
    // A leading ':' is the Twitch-native trigger convention: ':Pog' should match
    // the emote 'Pog' (no emote name contains a colon) and float Twitch emotes to
    // the front of the carousel. A trailing ':' (':Pog:') is tolerated too. The
    // '@' (chatter) and ':' (Twitch-first) prefixes are mutually exclusive.
    const isColonQuery = q.startsWith(':');
    const stripAt = isAtQuery
      ? q.slice(1)
      : isColonQuery
        ? q.slice(1).replace(/:$/, '')
        : q;
    const test = (token: string) => {
      const t = token.toLowerCase();
      return mode === 'starts_with' ? t.startsWith(stripAt) : t.includes(stripAt);
    };

    if (emotes && !isAtQuery && stripAt) {
      const favoriteIds = new Set(favoriteEmotes.map(f => f.id));
      // Walk providers in tier order so the seen-set drops cross-provider dupes
      // in favor of the higher-tier provider (e.g. a 7TV "Kappa" wins over the
      // Twitch one). A colon-prefixed query flips Twitch to the front so
      // Twitch-native / sub emotes lead, with the third-party sets as fallback.
      // The Kick slot is appended last: it's empty for Twitch (so tier indices
      // and ordering stay byte-identical there), and for a Kick channel its
      // native emotes become tab-completable alongside 7TV.
      const ordered: Array<[Emote['provider'], Emote[] | undefined]> = isColonQuery
        ? [
            ['twitch', emotes.twitch],
            ['7tv', emotes['7tv']],
            ['bttv', emotes.bttv],
            ['ffz', emotes.ffz],
            ['kick', emotes.kick],
            ['youtube', emotes.youtube],
          ]
        : [
            ['7tv', emotes['7tv']],
            ['bttv', emotes.bttv],
            ['ffz', emotes.ffz],
            ['twitch', emotes.twitch],
            ['kick', emotes.kick],
            ['youtube', emotes.youtube],
          ];
      const tierOf = (provider: Emote['provider']) =>
        ordered.findIndex(([p]) => p === provider);
      for (const [provider, list] of ordered) {
        if (!list) continue;
        for (const e of list) {
          const key = e.name.toLowerCase();
          if (seen.has(key)) continue;
          if (!test(e.name)) continue;
          // Subscriber-only FFZ effects are not offered to non-subscribers
          // (they still render in incoming messages).
          if (e.ffzSubOnly && !useAppStore.getState().ffzIsSubwoofer) continue;
          seen.add(key);
          ranked.push({
            providerTier: tierOf(provider),
            favoriteRank: favoriteIds.has(e.id) ? 0 : 1,
            item: {
              name: e.name,
              priority: tierOf(provider),
              emote: {
                id: e.id,
                name: e.name,
                url: e.url,
                localUrl: e.localUrl,
                provider: e.provider,
                isZeroWidth: e.isZeroWidth,
                modifierFlags: e.modifierFlags,
                ffzSubOnly: e.ffzSubOnly,
              },
            },
          });
        }
      }
    }

    if (includeChatters && !isColonQuery) {
      const chatters = getMatchingUsers(stripAt);
      for (const u of chatters) {
        const dn = u.displayName || u.username;
        const key = dn.toLowerCase();
        if (seen.has(key)) continue;
        if (!test(dn) && !test(u.username)) continue;
        seen.add(key);
        ranked.push({
          providerTier: 4, // chatters always after every emote provider
          favoriteRank: 1,
          item: {
            name: (isAtQuery ? '@' : '') + dn,
            priority: 4,
            chatter: { username: u.username, displayName: dn },
          },
        });
      }
    }

    ranked.sort((a, b) => {
      if (a.providerTier !== b.providerTier) return a.providerTier - b.providerTier;
      if (a.favoriteRank !== b.favoriteRank) return a.favoriteRank - b.favoriteRank;
      return a.item.name.localeCompare(b.item.name);
    });

    return ranked.slice(0, TAB_MATCH_LIMIT).map(r => r.item);
  }, [emotes, favoriteEmotes, getMatchingUsers, settings.chat_input]);

  /**
   * Replace the word at the cursor with the next (or previous, if backwards)
   * matching token. Maintains tab state across consecutive Tab presses so
   * pressing Tab again cycles through the same candidate list. State invalidates
   * when the user edits the input or moves the cursor elsewhere.
   */
  const handleEmoteTabPress = useCallback((isBackwards: boolean) => {
    if (!(settings.chat_input?.emote_tab_complete_enabled ?? true)) return false;
    const textarea = inputRef.current;
    if (!textarea) return false;
    const cursor = textarea.selectionStart ?? messageInput.length;
    if (textarea.selectionEnd !== cursor) return false; // active selection -> bail

    let matches: EmoteTabCandidate[];
    let nextIndex: number;
    let originalStart: number;
    let originalQuery: string;

    const stateMatchesCurrent =
      emoteTabState &&
      emoteTabState.expectedValue === messageInput &&
      emoteTabState.expectedCursor === cursor &&
      emoteTabState.matches.length > 0;

    if (stateMatchesCurrent) {
      matches = emoteTabState!.matches;
      nextIndex = isBackwards ? emoteTabState!.index - 1 : emoteTabState!.index + 1;
      nextIndex = ((nextIndex % matches.length) + matches.length) % matches.length;
      originalStart = emoteTabState!.originalStart;
      originalQuery = emoteTabState!.originalQuery;
    } else {
      const [ws, we] = getWordRange(messageInput, cursor);
      if (cursor === ws) return false;
      const word = messageInput.slice(ws, we);
      if (!word || word === ' ') return false;
      matches = getMatchingEmoteTokens(word);
      if (matches.length === 0) return false;
      nextIndex = 0;
      originalStart = ws;
      originalQuery = word;
    }

    const match = matches[nextIndex];
    const prevTokenLen = stateMatchesCurrent ? emoteTabState!.currentLen : originalQuery.length;
    const before = messageInput.slice(0, originalStart);
    const after = messageInput.slice(originalStart + prevTokenLen);
    // Only add a trailing space if there isn't already one immediately after.
    const addTrailingSpace = after.length === 0 || after[0] !== ' ';
    const replacement = match.name + (addTrailingSpace ? ' ' : '');
    const newValue = before + replacement + after;
    const newCursor = originalStart + replacement.length;

    setMessageInput(newValue);

    setEmoteTabState({
      matches,
      index: nextIndex,
      expectedCursor: newCursor,
      expectedValue: newValue,
      originalStart,
      originalQuery,
      currentLen: replacement.length,
    });

    setTimeout(() => {
      const ta = inputRef.current;
      if (ta) {
        ta.focus({ preventScroll: true });
        ta.setSelectionRange(newCursor, newCursor);
      }
    }, 0);

    return true;
  }, [emoteTabState, messageInput, getMatchingEmoteTokens, settings.chat_input]);

  // The handlers passed down to ChatMessageList are all useCallback'd. They feed
  // a memoized list of up to ~1150 rows, so an unstable identity here re-renders
  // every visible message on every ChatWidget render — including every keystroke
  // in the input below.
  const handleEmoteRightClick = useCallback((emoteName: string) => {
    setMessageInput(prev => {
      if (prev.trim()) return prev + (prev.endsWith(' ') ? '' : ' ') + emoteName + ' ';
      return emoteName + ' ';
    });
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  const handleUsernameRightClick = useCallback((messageId: string, username: string) => {
    setReplyingTo({ messageId, username });
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  const handleMessageCopy = useCallback((content: string) => {
    setMessageInput(content + ' ');
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  // Declared here, above the early returns below, so it can be a useCallback
  // (hooks can't live after a conditional return).
  const handleUsernameClick = useCallback(async (userId: string, username: string, displayName: string, color: string, badges: Array<{ key: string; info: any }>, event: React.MouseEvent, provider: ProviderId = 'twitch') => {
    // Placement lives in openProfilePopup, which is the single implementation. This
    // used to hand-roll its own copy that added the main window's PHYSICAL position
    // to the click's LOGICAL clientX and passed the result as a logical window
    // coordinate — so on a scaled monitor Tauri multiplied the already-inflated
    // number by the scale factor again and threw the card onto whatever display sat
    // to the right. The shared helper converts units properly and clamps the card
    // inside the monitor the cursor is actually on.
    const opened = await openProfilePopup({
      userId,
      username,
      displayName,
      color,
      badges,
      channelId: currentStream?.user_id || '',
      channelName: currentStream?.user_login || '',
      isModerator,
      provider,
      clientX: event.clientX,
      clientY: event.clientY,
    });
    if (!opened) {
      setSelectedUser({ userId, username, displayName, color, badges, position: { x: event.clientX, y: event.clientY } });
    }
  }, [currentStream?.user_id, currentStream?.user_login, isModerator]);
  // Refresh the latest-handler ref each render so window-event callers (e.g. the
  // /usercard and /user commands) invoke the current closure.
  handleUsernameClickRef.current = handleUsernameClick;

  const handlePauseIntent = useCallback(() => {
    // Fired the instant the user scrolls up by any amount. This
    // is the primary pause path — no distance threshold, just
    // like Twitch: scroll up at all and chat pauses. It is safe
    // from layout jolts because only a real wheel/touch gesture
    // reaches here (a tall message moving the scrollbar does not).
    const now = Date.now();
    if (now - mountTimeRef.current < 2000) return;       // initial layout settle
    if (now - lastResumeTimeRef.current < 1000) return;  // post-resume inertia
    if (now - lastNavigationTimeRef.current < 1000) return; // scrollToMessage animation
    setChatPaused(true);
  }, [setChatPaused]);

  const handleListScroll = useCallback((distanceToBottom: number, isUserScroll: boolean) => {
    // Debug logging - to be removed after verification
    if (isUserScroll && distanceToBottom > 150) {
      Logger.debug('[ChatWidget] Potential Pause Trigger:', { distanceToBottom, isUserScroll, isPaused });
    }

    // Grace Periods:
    // 1. Initial Load: Ignore first 2s to allow layout stabilization
    // 2. Resume: Ignore first 1s after clicking resume
    const now = Date.now();
    if (now - mountTimeRef.current < 2000) return;
    if (now - lastResumeTimeRef.current < 1000) return;
    if (now - lastNavigationTimeRef.current < 1000) return; // Skip during scrollToMessage animation

    // User scrolled up (away from bottom) - pause chat
    // STRICT: Only pause if Child component confirms it was a USER interaction (isUserScroll)
    if (isUserScroll && distanceToBottom > 150 && !isPaused) {
      setChatPaused(true);
    }
    // User scrolled back to bottom while paused - auto-resume.
    // Guarded (no force): the settle window absorbs the rapid
    // toggles a fast chat would otherwise produce here.
    else if (isPaused && distanceToBottom < 30) {
      setChatPaused(false, { scrollToBottom: true });
    }
  }, [isPaused, setChatPaused]);




  if (!currentStream) {
    return (
      <div className="h-full bg-secondary backdrop-blur-md flex items-center justify-center p-4">
        <p className="text-textSecondary">No stream selected</p>
      </div>
    );
  }

  // isConnected (meta) drives the exit from this screen; the length read is a
  // one-shot getState peek, which is enough because connect always re-renders.
  const showLoadingScreen =
    !isConnected &&
    (panelChannelKey
      ? useChatConnectionStore.getState().channels.get(panelChannelKey)?.messages.length ?? 0
      : 0) === 0;
  if (showLoadingScreen) {
    return (
      <div className="h-full bg-secondary backdrop-blur-md flex items-center justify-center p-4">
        <p className="text-textSecondary">Connecting to chat...</p>
      </div>
    );
  }

  return (
    <>
      <div ref={setChatContainerEl} className="h-full bg-secondary overflow-hidden flex flex-col relative">
        {/* Floating cards at the top of chat. Twitch allows a poll and a
            prediction at once, so they stack in one column rather than both
            pinning themselves to the same box. Order is the user's choice. */}
        <ChatOverlayStack isHypeTrainActive={!!currentHypeTrain}>
          {(settings.chat_overlay_order === 'poll-first'
            ? (['poll', 'prediction'] as const)
            : (['prediction', 'poll'] as const)
          ).map((kind) => {
            // Polls and predictions are Twitch products driven by EventSub. On a
            // provider stream both overlays were still mounting and watching a
            // Kick slug / YouTube UC id that can never produce an event.
            if (!isTwitch) return null;
            if (kind === 'poll') {
              if (settings.show_polls === false) return null;
              return (
                <PollOverlay
                  key="poll"
                  channelId={currentStream?.user_id}
                  channelLogin={currentStream?.user_login}
                />
              );
            }
            if (settings.show_predictions === false) return null;
            return (
              <PredictionOverlay
                key="prediction"
                channelId={currentStream?.user_id}
                channelLogin={currentStream?.user_login}
              />
            );
          })}
        </ChatOverlayStack>

        {/* Chat header - transforms when Hype Train active */}
        {/* flex-col-reverse keeps the stream-info row on top while the hype bar
            (declared first below) renders underneath it */}
        <div className={`absolute top-0 left-0 right-0 px-3 py-2 border-b backdrop-blur-ultra z-10 pointer-events-none shadow-lg overflow-hidden flex flex-col-reverse ${
          isSharedChat && !currentHypeTrain ? 'iridescent-border' : 'border-borderSubtle'
        }`} style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 90%, transparent)' }}>
          {currentHypeTrain && (
            <HypeTrainBanner
              train={currentHypeTrain}
              confettiTarget={chatContainerEl}
              onExpire={() => {
                if (!channelOverride) useAppStore.getState().setCurrentHypeTrain(null);
              }}
            />
          )}
          {/* Stream-info header, always visible; sits as the top row, above the hype bar */}
          <div className="relative z-10">
              <div
                className={`flex items-center gap-2 ${pinnedMessages.length > 0 ? 'pointer-events-auto cursor-pointer' : ''}`}
                onClick={pinnedMessages.length > 0 ? () => {
                  const currentPinId = pinnedMessages[0]?.id || '';
                  const next = !isPinnedExpanded;
                  setIsPinnedExpanded(next);
                  if (next) {
                    seenPinIdRef.current = currentPinId;
                  }
                } : undefined}
              >
                <div className={`w-2 h-2 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : 'bg-red-400'}`}></div>
                {/* MultiChat panes: show which platform this chat is from, so split
                    columns are identifiable at a glance. */}
                {channelOverride && !isMainSurface && (
                  <ProviderLogo provider={channelOverride.provider ?? 'twitch'} size={13} className="flex-shrink-0" />
                )}
                {/* Chat status label. The STREAM CHAT <-> ABOUT carousel toggle was
                    retired: the channel About is now reached by scrolling down on
                    the player (ChannelAboutReveal). */}
                <AnimatePresence mode="wait" initial={false}>
                  {activeView === 'modroom' ? (
                    <motion.div
                      key="modroom-header"
                      initial={{ opacity: 0, y: -4 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: 4 }}
                      transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
                      className="flex min-w-0 items-center gap-2 whitespace-nowrap"
                    >
                      {modRoomStatus.connected ? (
                        <Tooltip
                          side="bottom"
                          content={
                            modRoomStatus.members.length > 0 ? (
                              <div className="flex flex-col gap-1 py-0.5">
                                {modRoomStatus.members.map((mm) => (
                                  <span key={mm.userId} className="flex items-center gap-1.5 text-xs">
                                    <span
                                      className={`h-1.5 w-1.5 shrink-0 rounded-full ${mm.active !== false ? 'bg-accent' : 'bg-white/25'}`}
                                    />
                                    <span className={mm.role === 'broadcaster' ? 'text-[#f0c674]' : 'text-textPrimary'}>
                                      {mm.login}
                                    </span>
                                  </span>
                                ))}
                              </div>
                            ) : (
                              'Just you'
                            )
                          }
                        >
                          <span className="flex cursor-default items-center whitespace-nowrap text-xs text-textSecondary">
                            <motion.span
                              key={modRoomStatus.memberCount}
                              initial={{ opacity: 0, y: -3, scale: 0.7 }}
                              animate={{ opacity: 1, y: 0, scale: 1 }}
                              transition={{ type: 'spring', stiffness: 520, damping: 26 }}
                              className="mr-1 inline-block font-semibold text-textPrimary"
                            >
                              {modRoomStatus.memberCount || 1}
                            </motion.span>
                            in the room
                          </span>
                        </Tooltip>
                      ) : (
                        <span className="text-xs text-textSecondary">Connecting...</span>
                      )}
                    </motion.div>
                  ) : (
                    <motion.p
                      key="chat-header"
                      initial={{ opacity: 0, y: -4 }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: 4 }}
                      transition={{ duration: 0.16, ease: [0.16, 1, 0.3, 1] }}
                      className={`min-w-0 truncate text-xs font-semibold leading-4 whitespace-nowrap ${isSharedChat ? 'iridescent-title' : 'text-textPrimary'}`}
                    >
                      {!isConnected
                        ? 'DISCONNECTED'
                        : channelOverride && !isMainSurface
                          ? channelOverride.user_name || channelOverride.user_login || 'STREAM CHAT'
                          : isSharedChat
                            ? 'SHARED STREAM CHAT'
                            : currentMediaType === 'offline_chat'
                              ? 'OFFLINE CHAT'
                              : 'STREAM CHAT'}
                    </motion.p>
                  )}
                </AnimatePresence>
                {/* MultiChat panes have no player, so surface the live title/game
                    here. The MAIN window does have one — showing them again in the
                    chat header just repeats what is already on screen, which is why
                    Twitch's chat header carries neither. */}
                {channelOverride && !isMainSurface && currentStream?.title && activeView !== 'modroom' && (
                  <Tooltip content={currentStream.title} side="bottom">
                    <p className="min-w-0 flex-1 truncate text-[10px] font-normal leading-4 text-textMuted">
                      {currentStream.game_name ? `${currentStream.game_name} · ` : ''}
                      {currentStream.title}
                    </p>
                  </Tooltip>
                )}
                <div className="flex items-center gap-3 ml-auto shrink-0">
                  {/* Compact Chat / Mod Room toggle: the active pill slides between
                      the two with a spring (magnetic). Shown for moderators, using
                      the optimistic eligibility so it appears instantly on revisit. */}
                  {modRoomEligible && currentStream && (
                    <div
                      className="pointer-events-auto relative order-last flex items-center rounded-full p-0.5"
                      style={{ background: 'rgba(255,255,255,0.06)', boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.07)' }}
                    >
                      {(
                        [
                          { key: 'chat', label: 'Chat', active: activeView !== 'modroom' },
                          { key: 'modroom', label: 'Mods', active: activeView === 'modroom' },
                        ] as const
                      ).map((seg) => (
                        <button
                          key={seg.key}
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            setActiveView(seg.key);
                          }}
                          className={`relative rounded-full px-3.5 py-1 text-xs font-semibold transition-colors ${seg.active ? 'text-textPrimary' : 'text-textSecondary hover:text-textPrimary'}`}
                        >
                          {seg.active && (
                            <motion.span
                              layoutId="modroom-toggle-pill"
                              className="absolute inset-0 rounded-full bg-white/[0.13]"
                              style={{ boxShadow: 'inset 0 0 0 1px rgba(255,255,255,0.09)' }}
                              transition={{ type: 'spring', stiffness: 480, damping: 28 }}
                            />
                          )}
                          <span className="relative z-10">{seg.label}</span>
                          {/* Unread chip: mentions turn it red so a ping is
                              visible from the Chat view without opening the room. */}
                          {seg.key === 'modroom' && !seg.active && modRoomUnread > 0 && (
                            <span
                              className={`relative z-10 ml-1 inline-flex min-w-[15px] items-center justify-center rounded-full px-1 text-[9px] font-bold leading-[15px] ${
                                modRoomMentions > 0 ? 'bg-red-500/85 text-white' : 'bg-accent/25 text-accent'
                              }`}
                            >
                              {modRoomUnread > 9 ? '9+' : modRoomUnread}
                            </span>
                          )}
                        </button>
                      ))}
                    </div>
                  )}
                  {/* In the mod room: the premium encrypted badge takes the place of
                      the viewers / uptime / pop-out cluster. */}
                  {activeView === 'modroom' && (
                    <motion.span
                      initial={{ opacity: 0, scale: 0.9 }}
                      animate={{ opacity: 1, scale: 1 }}
                      transition={{ type: 'spring', stiffness: 520, damping: 30 }}
                      className="inline-flex select-none items-center gap-1 rounded-full px-2 py-[3px] text-[11px] font-semibold tracking-tight text-emerald-300"
                      style={{
                        background: 'linear-gradient(180deg, rgba(16,185,129,0.16), rgba(16,185,129,0.08))',
                        boxShadow: 'inset 0 0 0 1px rgba(16,185,129,0.28), inset 0 1px 0 rgba(255,255,255,0.06)',
                      }}
                    >
                      <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                        <rect x="3" y="11" width="18" height="11" rx="2" />
                        <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                      </svg>
                      Encrypted
                    </motion.span>
                  )}
                  {activeView !== 'modroom' && (
                    <>
                  {/* Saved message filter for this pane (Chat settings). */}
                  {filterOptions.length > 1 && (
                    <Dropdown
                      value={filterId ?? ''}
                      options={filterOptions}
                      onChange={(v) => setFilterId(v ? String(v) : null)}
                      leadingIcon={<Filter size={11} />}
                      className={`pointer-events-auto h-5 px-1.5 text-[11px] ${filterId ? 'text-accent' : 'text-textSecondary'}`}
                      align="right"
                      ariaLabel="Message filter"
                    />
                  )}
                  {/* Viewers list — the official chatters roster grouped by role.
                      Mod/broadcaster only (Helix Get Chatters requires it), so the
                      toggle is hidden on channels the user doesn't moderate.
                      Twitch-only: Helix is the only roster source, so on a
                      provider channel the id would be for a different platform. */}
                  {isModerator && isTwitch && currentStream && (
                    <Tooltip content={activeView === 'viewers' ? 'Back to chat' : 'Viewers'} side="top">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setActiveView(activeView === 'viewers' ? 'chat' : 'viewers');
                        }}
                        className={`pointer-events-auto grid h-5 w-5 place-items-center rounded transition-colors hover:bg-surface-hover hover:text-textPrimary ${activeView === 'viewers' ? 'text-accent' : 'text-textSecondary'}`}
                        aria-label="Viewers list"
                      >
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
                          <circle cx="9" cy="7" r="4" />
                          <path d="M23 21v-2a4 4 0 0 0-3-3.87" />
                          <path d="M16 3.13a4 4 0 0 1 0 7.75" />
                        </svg>
                      </button>
                    </Tooltip>
                  )}
                  {/* Pop out chat — opens a separate StreamNook MultiChat window
                      pre-loaded with the currently watched channel. The window
                      survives the main app's lifecycle inside one Tauri process.
                      Hidden when ChatWidget is already inside a popout (channelOverride
                      set) — popping out of a popout would be confusing. */}
                  {currentStream && !channelOverride && (
                    <Tooltip content={isMultiNookActive && slots.length > 1 ? 'Pop out all chats' : 'Pop out chat'} side="top">
                      <button
                        type="button"
                        onClick={async (e) => {
                          e.stopPropagation();
                          try {
                            const { openMultiChatWindow } = await import('../utils/multichatWindow');
                            const mn = usemultiNookStore.getState();
                            if (mn.isMultiNookActive && mn.slots.length > 1) {
                              // In MultiNook with multiple streams: pop out ALL of them into
                              // a FRESH MultiChat window (replace any tabs that were open).
                              // The IRC bridge is shared and these channels are already
                              // joined (MultiNook keeps every tile connected), so the popout
                              // attaches to the live session instead of reloading it.
                              await openMultiChatWindow({
                                replace: true,
                                channels: mn.slots.map((s) => ({
                                  channel: s.channelLogin,
                                  channelId: s.channelId ?? null,
                                  channelName: s.channelName ?? s.channelLogin,
                                })),
                              });
                              // Chat now lives in the popout — hide the in-grid chat panel to
                              // reclaim space. (Re-show any time via the toolbar chat toggle.)
                              const after = usemultiNookStore.getState();
                              if (!after.isChatHidden) after.toggleChatHidden();
                            } else {
                              await openMultiChatWindow({
                                channel: currentStream.user_login,
                                channelId: currentStream.user_id || undefined,
                                channelName: currentStream.user_name || undefined,
                              });
                            }
                          } catch (err) {
                            Logger.error('[ChatWidget] Pop out chat failed:', err);
                          }
                        }}
                        className="pointer-events-auto grid h-5 w-5 place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
                        aria-label="Pop out chat"
                      >
                        <svg
                          width="12"
                          height="12"
                          viewBox="0 0 16 16"
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="1.5"
                          strokeLinecap="round"
                          strokeLinejoin="round"
                        >
                          <path d="M9 2h5v5" />
                          <path d="M14 2L7 9" />
                          <path d="M13 9v4a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h4" />
                        </svg>
                      </button>
                    </Tooltip>
                  )}
                  {!channelOverride && isTwitch && currentStream && (
                    <Tooltip content="Float chat as a see-through window over other apps (its own renderer, about 150 MB)" side="top">
                      <button
                        type="button"
                        onClick={async (e) => {
                          e.stopPropagation();
                          try {
                            const { openChatOverlayWindow } = await import('../utils/chatOverlayWindow');
                            await openChatOverlayWindow({
                              channel: currentStream.user_login,
                              channelId: currentStream.user_id || undefined,
                              channelName: currentStream.user_name || undefined,
                            });
                          } catch (err) {
                            Logger.error('[ChatWidget] Open chat overlay failed:', err);
                          }
                        }}
                        className="pointer-events-auto grid h-5 w-5 place-items-center rounded text-textSecondary transition-colors hover:bg-surface-hover hover:text-textPrimary"
                        aria-label="Float chat as overlay"
                      >
                        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                          <rect x="2" y="2" width="9" height="9" rx="1.5" />
                          <path d="M6 14h7.5a.5.5 0 0 0 .5-.5V6" strokeDasharray="2 1.5" />
                        </svg>
                      </button>
                    </Tooltip>
                  )}
                  {viewerCount !== null && !streamerModeActive && (
                    <div className="flex items-center gap-1">
                      <svg className="w-3 h-3 text-textSecondary" fill="currentColor" viewBox="0 0 20 20"><path d="M10 12a2 2 0 100-4 2 2 0 000 4z" /><path fillRule="evenodd" d="M.458 10C1.732 5.943 5.522 3 10 3s8.268 2.943 9.542 7c-1.274 4.057-5.064 7-9.542 7S1.732 14.057.458 10zM14 10a4 4 0 11-8 0 4 4 0 018 0z" clipRule="evenodd" /></svg>
                      <span className="text-xs text-textSecondary">{viewerCount.toLocaleString()}</span>
                    </div>
                  )}
                  {currentStream?.started_at && (
                    <div className="flex items-center gap-1">
                      <svg className="w-3 h-3 text-textSecondary" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z" /></svg>
                      <span id="stream-uptime-display" className="text-xs text-textSecondary">{streamUptimeRef.current}</span>
                    </div>
                  )}
                  {/* Pin icon + chevron indicator */}
                  {pinnedMessages.length > 0 && (() => {
                    const currentPinId = pinnedMessages[0]?.id || '';
                    const isUnseen = currentPinId !== seenPinIdRef.current;
                    return (
                      <div className="flex items-center gap-1">
                        <svg className={`w-3.5 h-3.5 text-accent ${isUnseen ? 'animate-pulse drop-shadow-[0_0_4px_var(--color-accent)]' : ''}`} fill="currentColor" viewBox="0 0 16 16">
                          <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z"/>
                        </svg>
                        <svg className={`w-2.5 h-2.5 text-textSecondary transition-transform duration-200 ${isPinnedExpanded ? 'rotate-180' : ''}`} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                        </svg>
                      </div>
                    );
                  })()}
                    </>
                  )}
                </div>
              </div>
              {/* Pinned messages are now rendered in a floating modal outside the header */}
          </div>
        </div>

        {/* Free Floating Pinned Message Modal */}
        {pinnedMessages.length > 0 && (
          <div className="absolute left-3 right-3 z-[15] pointer-events-none flex flex-col items-center"
            style={{
              top: currentHypeTrain ? '80px' : '48px', // Positioning based on header height
            }}>
            <AnimatePresence>
              {isPinnedExpanded && (
                <motion.div
                  initial={{ opacity: 0, y: -10, scale: 0.96, height: 0 }}
                  animate={{ opacity: 1, y: 0, scale: 1, height: 'auto' }}
                  exit={{ opacity: 0, y: -10, scale: 0.96, height: 0 }}
                  transition={{ duration: 0.3, ease: [0.175, 0.885, 0.32, 1.2] }}
                  className="sn-popover w-full max-w-sm mt-2 pointer-events-auto overflow-hidden origin-top"
                >
                {pinnedMessages.map((pin, i) => {
                  // Same link-preview pipeline the main chat uses, applied to the
                  // pinned message.
                  const previewsOn = settings.chat_design?.link_previews ?? true;
                  const keepLink = settings.chat_design?.link_preview_keep_link ?? false;
                  const previewItems = previewsOn
                    ? extractPreviewUrls(
                        pin.message_text,
                        2,
                        settings.chat_design?.link_preview_trusted_domains,
                      )
                    : [];
                  // In "clean" mode the inline link is suppressed for any trusted
                  // (auto-carded) URL so the card is the sole representation,
                  // matching chat. Untrusted links keep their inline link.
                  const stripTrail = (s: string) => s.replace(/[.,!?:;'")\]}>]+$/, '');
                  const suppressed = !keepLink
                    ? new Set(previewItems.filter((it) => it.trusted).map((it) => it.url))
                    : null;
                  const textParts = pin.message_text.split(/(https?:\/\/\S+)/g);
                  const hasVisibleBody = textParts.some((part) =>
                    /^https?:\/\//.test(part)
                      ? !suppressed?.has(stripTrail(part))
                      : part.trim().length > 0,
                  );
                  return (
                  <div
                    key={pin.id}
                    className={`relative p-3.5 ${i > 0 ? 'border-t border-white/[0.05]' : ''}`}
                  >
                    {/* Unpin (mod-only): clears the current mod pin from the
                        pinned message itself. */}
                    {isModerator && currentStream?.user_id && (
                      <Tooltip content="Unpin message" side="left">
                        <button
                          type="button"
                          onClick={async () => {
                            try {
                              await invoke('unpin_chat_message', { broadcasterId: currentStream.user_id, messageId: pin.message_id });
                              usePinStore.getState().requestRefresh();
                            } catch (err) {
                              Logger.error('[ChatWidget] Failed to unpin message:', err);
                            }
                          }}
                          className="absolute top-2 right-2 z-10 p-1.5 rounded-lg text-textSecondary/70 hover:text-red-400 hover:bg-red-500/15 transition-colors pointer-events-auto"
                          aria-label="Unpin message"
                        >
                          <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="2" x2="22" y1="2" y2="22"/><line x1="12" x2="12" y1="17" y2="22"/><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V17h12"/><path d="M15 9.34V6h1a2 2 0 0 0 0-4H7.89"/></svg>
                        </button>
                      </Tooltip>
                    )}
                    {/* Sender row: avatar + badges + name + message */}
                    <div className="flex items-start gap-3">
                      {/* Sender avatar */}
                      {pin.sender_avatar ? (
                        <img
                          src={pin.sender_avatar}
                          alt={pin.sender_name}
                          className="w-9 h-9 rounded-full flex-shrink-0 object-cover border border-white/5"
                        />
                      ) : (
                        <div
                          className="w-9 h-9 rounded-full flex-shrink-0 flex items-center justify-center text-[13px] font-bold text-white shadow-inner"
                          style={{ backgroundColor: pin.sender_color }}
                        >
                          {pin.sender_name.charAt(0).toUpperCase()}
                        </div>
                      )}
                      <div className="flex-1 min-w-0 pt-0.5">
                        {/* Name + badges row */}
                        <div className="flex items-center gap-1.5 flex-wrap">
                          {/* Badges — resolved from in-memory badge cache */}
                          {(() => {
                            const badgeStr = pin.sender_badges?.map(b => `${b.set_id}/${b.version}`).join(',') || '';
                            if (!badgeStr) return null;
                            const resolved = parseBadges(badgeStr, currentStream?.user_id);
                            return resolved.map((badge: { key: string; info: { image_url_1x?: string; image_url_2x?: string; title?: string } | null }, bi: number) => (
                              badge.info?.image_url_1x ? (
                                <Tooltip key={`${badge.key}-${bi}`} content={badge.info.title || badge.key}>
                                <img
                                  src={badge.info.image_url_2x || badge.info.image_url_1x}
                                  alt={badge.info.title || badge.key}
                                  className="w-4 h-4 flex-shrink-0 drop-shadow-sm"
                                />
                                </Tooltip>
                              ) : null
                            ));
                          })()}
                          <span
                            className="text-sm font-bold tracking-tight"
                            style={{ color: pin.sender_color, textShadow: '0 1px 2px rgba(0,0,0,0.8)' }}
                          >
                            {pin.sender_name}
                          </span>
                        </div>
                        {/* Message text with clickable links. Trusted carded
                            links are suppressed in clean mode (the card is the
                            link); the whole paragraph is skipped if nothing
                            visible remains (e.g. a pin that's only a link). */}
                        {hasVisibleBody && (
                          <p className="text-[13.5px] text-textPrimary/95 mt-1.5 break-words font-medium" style={{ lineHeight: '1.5' }}>
                            {textParts.map((part, index) =>
                              /^https?:\/\//.test(part) ? (
                                suppressed?.has(stripTrail(part)) ? null : (
                                  <a
                                    key={index}
                                    className="text-accent/90 hover:text-accent hover:underline pointer-events-auto cursor-pointer transition-colors"
                                    onClick={() => {
                                      import('@tauri-apps/plugin-shell').then(({ open }) => open(part));
                                    }}
                                  >
                                    {part.length > 50 ? part.slice(0, 50) + '…' : part}
                                  </a>
                                )
                              ) : (
                                <span key={index}>
                                  {renderPinTextWithEmotes(part, `pin-${pin.id}-${index}`, 'h-6 max-w-[112px] -my-1')}
                                </span>
                              )
                            )}
                          </p>
                        )}
                        {/* Inline link preview cards. In clean mode the inline
                            link above is suppressed and showChip lets a failed
                            trusted preview fall back to a link chip so the link is
                            never lost. Untrusted links render a click-to-load
                            chip (with the always-trust shield). */}
                        {previewItems.length > 0 && (
                          <div className="mt-2 flex flex-col items-start gap-1">
                            {previewItems.map((it) => (
                              <LinkPreviewCard
                                key={it.url}
                                url={it.url}
                                trusted={it.trusted}
                                showChip={it.trusted && !keepLink}
                              />
                            ))}
                          </div>
                        )}
                      </div>
                    </div>
                    {/* Pinned by footer — visually separated */}
                    {pin.pinned_by && (
                      <div className="flex items-center gap-2 mt-3 pt-2.5 border-t border-white/[0.04] ml-[48px]">
                        <svg className="w-3 h-3 text-accent" fill="currentColor" viewBox="0 0 16 16">
                          <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z"/>
                        </svg>
                        {pin.pinned_by_avatar && (
                          <img
                            src={pin.pinned_by_avatar}
                            alt={pin.pinned_by}
                            className="w-4 h-4 rounded-full object-cover border border-white/10"
                          />
                        )}
                        <span className="text-[11px] text-textSecondary uppercase tracking-wide font-medium">
                          Pinned by <span className="font-bold text-textPrimary/80 ml-0.5 capitalize">{pin.pinned_by}</span>
                        </span>
                      </div>
                    )}
                  </div>
                  );
                })}
                </motion.div>
              )}
            </AnimatePresence>
            {/* Collapsed state: a thin one-line bar (pin + sender + truncated
                text) instead of hiding the pin entirely. Click to expand.
                Opt-out via chat_design.pinned_collapsed_style === 'hidden'. */}
            <AnimatePresence>
              {!isPinnedExpanded && (settings.chat_design?.pinned_collapsed_style ?? 'bar') !== 'hidden' && (() => {
                const pin = pinnedMessages[0];
                return (
                  <motion.button
                    type="button"
                    initial={{ opacity: 0, y: -8 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -8 }}
                    transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
                    onClick={() => {
                      setIsPinnedExpanded(true);
                      seenPinIdRef.current = pin.id;
                    }}
                    className="sn-popover sn-popover-hover group w-full max-w-sm mt-2 pointer-events-auto overflow-hidden flex items-center gap-2.5 px-3.5 py-2.5 text-left transition-colors"
                  >
                    <svg className="w-3.5 h-3.5 text-accent flex-shrink-0" fill="currentColor" viewBox="0 0 16 16">
                      <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z"/>
                    </svg>
                    <span
                      className="text-[13px] font-bold flex-shrink-0 max-w-[35%] truncate"
                      style={{ color: pin.sender_color, textShadow: '0 1px 2px rgba(0,0,0,0.8)' }}
                    >
                      {pin.sender_name}
                    </span>
                    <span className="text-[13px] text-textPrimary/85 truncate min-w-0 flex-1">
                      {renderPinTextWithEmotes(pin.message_text, `pinbar-${pin.id}`, 'h-5 max-w-[80px] -my-0.5')}
                    </span>
                    {pinnedMessages.length > 1 && (
                      <span className="text-[11px] font-semibold text-textSecondary flex-shrink-0 tabular-nums">
                        +{pinnedMessages.length - 1}
                      </span>
                    )}
                    {/* Expand chevron */}
                    <svg className="w-3 h-3 text-textSecondary/60 group-hover:text-textSecondary flex-shrink-0 transition-colors" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M19 9l-7 7-7-7" />
                    </svg>
                  </motion.button>
                );
              })()}
            </AnimatePresence>
          </div>
        )}

        {/* Staging area removed - using direct rendering with ResizeObserver */}

        {/* Viewers list - replaces chat when active */}
        {activeView === 'viewers' && currentStream && (
          <div className={`flex-1 overflow-hidden animate-panel-slide-up ${currentHypeTrain ? 'pt-24' : 'pt-10'}`}>
            <ViewersPanel
              key={currentStream.user_id}
              broadcasterId={currentStream.user_id}
              channelLogin={currentStream.user_login}
              onUsernameClick={handleUsernameClick}
            />
          </div>
        )}

        <AnimatePresence mode="popLayout">
          {activeView === 'modroom' && currentStream && (
            <motion.div
              key="modroom-pane"
              className={`flex-1 overflow-hidden ${currentHypeTrain ? 'pt-24' : 'pt-10'}`}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 8 }}
              transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
            >
              <ModRoomPane
                key={currentStream.user_id}
                channelId={currentStream.user_id}
                channelLogin={currentStream.user_login}
                emotes={emotes}
                onStatus={setModRoomStatus}
                onUsernameClick={(login, userId, event) =>
                  handleUsernameClick(userId, login, login, '', [], event)
                }
              />
            </motion.div>
          )}
        </AnimatePresence>

        {activeView === 'chat' && searchOpen && (
          <ChatSearchBar
            channelKey={panelChannelKey}
            onJumpTo={(id) => scrollToMessage(id, { highlight: true, align: 'center' })}
            onClose={() => setSearchOpen(false)}
          />
        )}
        {/* Chat messages area - flex-1 to take remaining space */}
        {activeView === 'chat' && (
          <ChatMessagesPanel
            channelKey={panelChannelKey}
            filterId={filterId}
            provider={provider}
            providerKey={providerKey}
            stream={currentStream}
            kickAccountName={kickAccountName}
            onKickModeratorDetected={onKickModeratorDetected}
            setIsSharedChat={setIsSharedChat}
            userMessageHistoryRef={userMessageHistory}
            sharedRoomsRef={sharedRoomsSeenRef}
            processedIdsRef={processedMessageIdsRef}
            messagesRef={messagesRef}
            override={panelOverride}
            isPaused={isPaused}
            onPauseIntent={handlePauseIntent}
            onScroll={handleListScroll}
            onUsernameClick={handleUsernameClick}
            onReplyClick={handleReplyClick}
            onMessageCopy={handleMessageCopy}
            onEmoteRightClick={handleEmoteRightClick}
            onUsernameRightClick={handleUsernameRightClick}
            onBadgeClick={handleBadgeClick}
            highlightedMessageId={highlightedMessageId}
            modFocusId={modFocusId}
            hiddenMessageIds={locallyHiddenMessageIds}
            emotes={emotes}
            getMessageId={getMessageId}
            isModerator={isModerator}
            broadcasterId={currentStream?.user_id}
            hoveringRef={isHoveringChatRef}
          />
        )}
        {/* AutoMod held-message queue (moderators, Twitch). Rust owns the
            queue; this strip mirrors it and sends Allow / Deny to Helix. */}
        {activeView === 'chat' && isModerator && isTwitch && panelChannelKey && (
          <AutomodQueueStrip channel={panelChannelKey} />
        )}

        {/* Chat Paused indicator - positioned above input */}
        {activeView === 'chat' && isPaused && (
          <div className="absolute bottom-[60px] left-1/2 transform -translate-x-1/2 z-50 pointer-events-auto">
            <button onClick={handleResume} className="flex items-center gap-2 px-4 py-2 glass-button text-white text-sm font-medium rounded-full shadow-lg">
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" /></svg>
              <span>Chat Paused<PausedNewCount channelKey={panelOverride ? null : panelChannelKey} isPaused={isPaused} /></span>
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" /></svg>
            </button>
          </div>
        )}

        {/* Input container - static flex item at bottom (hidden when About view active) */}
        {activeView === 'chat' &&
        <div className="flex-shrink-0 border-t border-borderSubtle" style={{ backgroundColor: 'color-mix(in srgb, var(--color-background) 94%, transparent)' }}>
          <div className="p-2">
            <div className="relative">
              {/* channelId/channelLogin below are THIS widget's channel, not the
                  main window's. Reading `currentStream` meant a MultiChat pane (or
                  the main window watching a provider stream) loaded the emote set
                  for whatever the main player happened to be on — a different
                  channel, and on Kick a different platform entirely. */}
              <EmotePickerPanel
                open={showEmotePicker}
                onClose={() => setShowEmotePicker(false)}
                emotes={emotes}
                isTwitch={isTwitch}
                isKick={provider === 'kick'}
                isYouTube={provider === 'youtube'}
                channelId={channelOverride?.user_id ?? currentStream?.user_id}
                channelLogin={channelOverride?.user_login ?? currentStream?.user_login}
                isLoadingEmotes={isLoadingEmotes}
                channelNameCache={channelNameCache}
                onInsert={insertEmote}
                onManageEmotes={
                  // The overlay this opens is scoped to a Twitch user id, so it
                  // only ever made sense on a Twitch pane.
                  isTwitch
                    ? () => {
                        setShowEmotePicker(false);
                        openEmoteSets({ twitchId: currentStream?.user_id, tab: 'emotes' });
                      }
                    : undefined
                }
              />

              {/* / Command Autocomplete (Dominated Width) */}
              <CommandMenu
                open={showCommandMenu}
                onClose={() => setShowCommandMenu(false)}
                commands={browsableCommands}
                isModerator={isModerator}
                isBroadcaster={!!isBroadcaster}
                ignoreRef={commandMenuButtonRef}
                onPick={(cmd) => {
                  setShowCommandMenu(false);
                  insertCommand(cmd);
                }}
                onInsertText={(text) => {
                  setShowCommandMenu(false);
                  setShowCommandAutocomplete(false);
                  flowReplaceFromRef.current = null;
                  setMessageInput(text);
                  inputRef.current?.focus({ preventScroll: true });
                  setTimeout(() => inputRef.current?.setSelectionRange(text.length, text.length), 0);
                }}
              />
              <AnimatePresence>
                {showCommandAutocomplete && !showCommandMenu && (
                  <CommandAutocomplete
                    commands={matchingCommands}
                    selectedIndex={commandSelectedIndex}
                    onSelect={(cmd) => insertCommand(cmd)}
                    onSelectedIndexChange={setCommandSelectedIndex}
                  />
                )}
              </AnimatePresence>
              {/* Channel Points Menu - renders at full width like emote picker */}
              {composerKind && currentStream?.user_id && (
                <PollPredictionComposer
                  broadcasterId={currentStream.user_id}
                  initialKind={composerKind}
                  onClose={() => setComposerKind(null)}
                />
              )}
              {showChannelPointsMenu && currentStream && (
                <ChannelPointsMenu
                  channelLogin={currentStream.user_login}
                  channelId={currentStream.user_id}
                  currentBalance={channelPoints}
                  customPointsName={customPointsName}
                  customPointsIconUrl={customPointsIconUrl}
                  onClose={() => setShowChannelPointsMenu(false)}
                  onBalanceUpdate={fetchChannelPoints}
                  onEmotesChange={() => loadEmotes(currentStream.user_login, currentStream.user_id)}
                />
              )}
              {/* Resub Notification Banner - shows when user has a shareable resub */}
              {resubNotification && !resubDismissed && currentStream && (
                <ResubNotificationBanner
                  resubNotification={resubNotification}
                  channelLogin={currentStream.user_login}
                  isResubMode={isResubMode}
                  includeStreak={includeStreak}
                  onActivateShare={() => {
                    setIsResubMode(true);
                    setReplyingTo(null); // Clear reply mode if active
                    inputRef.current?.focus({ preventScroll: true });
                  }}
                  onDismiss={() => {
                    setResubDismissed(true);
                    setIsResubMode(false);
                  }}
                  onToggleStreak={() => setIncludeStreak(!includeStreak)}
                  onCancelShare={() => {
                    setIsResubMode(false);
                    setMessageInput('');
                  }}
                />
              )}
              {/* Watch Streak Banner - shows when user has a shareable watch streak */}
              {watchStreak && !watchStreakDismissed && currentStream && (
                <WatchStreakBanner
                  milestone={watchStreak}
                  isStreakMode={isWatchStreakMode}
                  onActivateShare={() => {
                    setIsWatchStreakMode(true);
                    setIsResubMode(false); // Can't be in both modes
                    setReplyingTo(null);
                    inputRef.current?.focus({ preventScroll: true });
                  }}
                  onDismiss={() => {
                    setWatchStreakDismissed(true);
                    setIsWatchStreakMode(false);
                  }}
                  onCancelShare={() => {
                    setIsWatchStreakMode(false);
                    setMessageInput('');
                  }}
                />
              )}
              {/* /nuke dry run. Transient — it only exists while the command is
                  in the box, so it can't add permanent height to this shelf. */}
              {nukePreview && (
                <div className="mb-2 flex items-center gap-2 px-3 py-2 bg-glass rounded-lg border border-borderSubtle">
                  <Zap
                    size={14}
                    className={`flex-shrink-0 ${nukePreview.messages > 0 ? 'text-warning' : 'text-textSecondary'}`}
                  />
                  <span className="text-xs text-textSecondary flex-1">
                    {nukePreview.messages === 0 ? (
                      'Nothing in the buffer matches that yet'
                    ) : (
                      <>
                        Would {nukePreview.action}{' '}
                        <span className="text-warning font-semibold">
                          {nukePreview.messages.toLocaleString()}
                        </span>{' '}
                        {nukePreview.messages === 1 ? 'message' : 'messages'} from{' '}
                        <span className="text-warning font-semibold">
                          {nukePreview.users.toLocaleString()}
                        </span>{' '}
                        {nukePreview.users === 1 ? 'person' : 'people'}
                      </>
                    )}
                  </span>
                </div>
              )}
              {/* Active chat modes on a provider channel. Twitch shows none of
                  these today, so this is provider-only. Informational: the
                  platform enforces the rules, and a refused send now explains
                  itself rather than being silently swallowed. */}
              {providerRoomModes.length > 0 && (
                <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 px-1 text-[11px] text-textSecondary">
                  {providerRoomModes.map((mode) => (
                    <span key={mode}>{mode}</span>
                  ))}
                </div>
              )}
              {replyingTo && !isResubMode && !isWatchStreakMode && (
                <div className="mb-2 flex items-center gap-2 px-3 py-2 bg-glass rounded-lg border border-borderSubtle">
                  <svg className="w-4 h-4 text-accent flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h10a8 8 0 018 8v2M3 10l6 6m-6-6l6-6" /></svg>
                  <span className="text-xs text-textSecondary flex-1">Replying to <span className="text-accent font-semibold">{replyingTo.username}</span></span>
                  <Tooltip content="Cancel reply" side="top">
                  <button onClick={() => setReplyingTo(null)} className="text-textSecondary hover:text-textPrimary transition-colors">
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" /></svg>
                  </button>
                  </Tooltip>
                </div>
              )}

              {/* VOD chat source toggle. Uses the app's standard segmented control
                  (recessed track + sliding thumb) so it matches Settings and the
                  rest of the app instead of a one-off style. */}
              {isVodReplay && (
                <div className="mb-2">
                  <SegmentedSelect
                    fullWidth
                    value={chatMode}
                    onChange={(v) => setChatMode(v)}
                    options={[
                      { value: 'replay', label: 'VOD Chat' },
                      { value: 'live', label: 'Live Chat' },
                    ]}
                  />
                </div>
              )}

              <div className="flex items-center gap-2 min-w-0">
                {/* Channel Points button. Opens the rewards menu, balance on
                    hover. While the watched channel has a bonus chest ready
                    and auto-claim is off, it becomes the claim control.
                    Stays visible while a chest is waiting even if the balance
                    is hidden, so hiding the number never costs you points. */}
                {(showPointsBalance || (availableClaim && !autoClaimWatching)) && (
                <div
                  ref={channelPointsRef}
                  className="relative flex-shrink-0 self-center flex items-center"
                  onMouseEnter={() => setChannelPointsHovered(true)}
                  onMouseLeave={() => setChannelPointsHovered(false)}
                >
                  {availableClaim && !autoClaimWatching ? (
                    <Tooltip content="Claim bonus points" side="top">
                      <button
                        onClick={() => claimWatchedChest(availableClaim.id, availableClaim.channelId)}
                        disabled={claimingChest}
                        aria-label="Claim channel points bonus"
                        className="chest-attention flex items-center justify-center w-9 h-9 text-accent-neon transition-opacity duration-200 disabled:opacity-50"
                      >
                        <svg width={18} height={18} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                          <path fillRule="evenodd" clipRule="evenodd" d="M2 22v-9l2.417-4.029A2 2 0 0 1 6.132 8h11.736a2 2 0 0 1 1.715.971L22 13v9H2Zm18-2v-5h-7v2h-2v-2H4v5h16Zm-2.132-10 1.8 3H4.332l1.8-3h11.736Z" />
                        </svg>
                      </button>
                    </Tooltip>
                  ) : (
                    <button
                      onClick={() => setShowChannelPointsMenu(!showChannelPointsMenu)}
                      className={`group flex items-center justify-center w-9 h-9 transition-all duration-200 ${showChannelPointsMenu ? 'text-accent-neon' : channelPoints !== null ? 'text-accent-neon' : 'text-textSecondary hover:text-accent-neon'}`}
                    >
                      {customPointsIconUrl ? (
                        <img
                          src={customPointsIconUrl}
                          alt={customPointsName || "Channel Points"}
                          className="w-[18px] h-[18px] transition-all duration-200 group-hover:drop-shadow-[0_0_6px_color-mix(in_srgb,var(--color-accent-neon)_85%,transparent)]"
                        />
                      ) : (
                        <ChannelPointsIcon size={18} className="transition-all duration-200 group-hover:drop-shadow-[0_0_6px_color-mix(in_srgb,var(--color-accent-neon)_85%,transparent)]" />
                      )}
                    </button>
                  )}
                  {/* "+N" pop after a chest collect, manual or auto */}
                  <AnimatePresence>
                    {claimCelebration !== null && (
                      <motion.span
                        initial={{ opacity: 0, x: '-50%', y: 4, scale: 0.85 }}
                        animate={{ opacity: 1, x: '-50%', y: -16, scale: 1 }}
                        exit={{ opacity: 0, x: '-50%', y: -24 }}
                        transition={{ duration: 0.8, ease: 'easeOut' }}
                        className="absolute left-1/2 top-0 text-[11px] font-semibold text-accent-neon pointer-events-none whitespace-nowrap select-none"
                      >
                        +{claimCelebration.toLocaleString()}
                      </motion.span>
                    )}
                  </AnimatePresence>
                  {/* Points tooltip - fixed position to escape overflow-hidden parents */}
                  {channelPointsHovered && !showChannelPointsMenu && !(availableClaim && !autoClaimWatching) && (
                    <ChannelPointsTooltip
                      anchorRef={channelPointsRef}
                      customPointsIconUrl={customPointsIconUrl}
                      customPointsName={customPointsName}
                      isLoadingChannelPoints={isLoadingChannelPoints}
                      channelPoints={channelPoints}
                    />
                  )}
                </div>
                )}
                {/* Poll / prediction builder. Twitch scopes both create
                    endpoints to the channel owner, so this only appears on
                    your own channel. */}
                {isBroadcaster && isTwitch && !isVodReplay && (
                  <Tooltip content="Start a poll or prediction" side="top">
                    <button
                      onClick={() => setComposerKind((k) => (k ? null : 'poll'))}
                      className={`flex items-center justify-center w-9 h-9 flex-shrink-0 self-center transition-colors duration-200 ${
                        composerKind ? 'text-accent-neon' : 'text-textSecondary hover:text-accent-neon'
                      }`}
                      aria-label="Start a poll or prediction"
                    >
                      <BarChart3 size={18} />
                    </button>
                  </Tooltip>
                )}
                {/* Drops automation button: only when a drops automation plugin is installed
                    and the current game has active drops. Without the plugin, core
                    earns natively on the watched channel with no automation control. */}
                {dropsCampaign && externalDropsProvider && (
                  <Tooltip content={isDropProgressing ? `Stop collecting drops for ${dropsCampaign.game_name}` : `Start collecting drops for ${dropsCampaign.game_name}`} side="top">
                  <button
                    onClick={handleToggleAutomation}
                    disabled={isLoadingDrops}
                    className={`group flex-shrink-0 flex items-center justify-center self-center w-9 h-9 transition-all duration-200 ${isDropProgressing
                      ? 'text-green-400 hover:text-red-400'
                      : 'text-textSecondary hover:text-accent'
                      }`}
                  >
                    <Pickaxe size={18} className={`transition-all duration-200 group-hover:drop-shadow-[0_0_6px_color-mix(in_srgb,var(--color-accent-neon)_85%,transparent)] ${isDropProgressing ? 'animate-pulse' : ''}`} />
                  </button>
                  </Tooltip>
                )}
                {/* Moderator Dashboard */}
                {isModerator && currentStream && (
                  <div className="flex-shrink-0 self-center z-20">
                    {/* currentStream is synthesized from channelOverride in a
                        MultiChat pane, so these are this pane's channel, not
                        the main window's. */}
                    <ModeratorMenu
                      broadcasterId={currentStream.user_id}
                      channelLogin={currentStream.user_login}
                      isBroadcaster={!!isBroadcaster}
                      roomState={roomState}
                    />
                  </div>
                )}
                {/* Input container with emoji button inset on the left */}
                <div className="relative flex-1 min-w-0 flex items-center">
                  {/* Command menu button, first inset on the left */}
                  {showCommandButton && (
                  <Tooltip content={showCommandMenu ? 'Close commands' : 'Commands'} side="top">
                  <button
                    ref={commandMenuButtonRef}
                    type="button"
                    aria-label="Chat commands"
                    aria-expanded={showCommandMenu}
                    onClick={() => {
                      const next = !showCommandMenu;
                      if (next) setShowEmotePicker(false);
                      setShowCommandMenu(next);
                    }}
                    className="group absolute left-1 top-1/2 -translate-y-1/2 z-10 flex items-center justify-center w-7 h-7 text-textSecondary hover:text-textPrimary transition-colors duration-200"
                  >
                    <SquareSlash
                      size={16}
                      className={`transition-all duration-200 group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)] ${showCommandMenu ? 'text-accent' : ''}`}
                    />
                  </button>
                  </Tooltip>
                  )}
                  {/* Emoji button — inset left inside the input */}
                  {showEmoteButton && (
                  <Tooltip content={showEmotePicker ? "Close Emotes" : "Emotes"} side="top">
                  <button
                    onClick={() => {
                      if (!showEmotePicker) setShowCommandMenu(false);
                      // Twitch only. A non-Twitch pane legitimately has an empty
                      // `twitch` slot, so without this gate every picker open on
                      // a Kick or YouTube pane refetched the MAIN window's Twitch
                      // emotes and flashed the panel's loading state over the
                      // grid it was supposed to be showing.
                      if (!showEmotePicker && isTwitch && emotes && emotes.twitch.length <= 15) {
                        Logger.warn('[ChatWidget] Detected fallback emotes only. Retrying fetch before opening picker...');
                        if (currentStream) {
                           loadEmotes(currentStream.user_login, currentStream.user_id);
                        }
                      }
                      setShowEmotePicker(!showEmotePicker);
                    }}
                    onMouseLeave={smiley.cycleEmoteSmiley}
                    className={`group absolute ${showCommandButton ? 'left-8' : 'left-1'} top-1/2 -translate-y-1/2 z-10 flex items-center justify-center w-7 h-7 text-textSecondary hover:text-textPrimary transition-colors duration-200`}
                  >
                    {showEmotePicker ? (
                      <svg className="w-4 h-4 transition-all duration-200 text-accent group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)]" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2.5} d="M6 18L18 6M6 6l12 12" /></svg>
                    ) : (
                      <img
                        src={getAppleEmojiUrl(smiley.currentSmiley)}
                        alt={smiley.currentSmiley}
                        draggable={false}
                        className={`w-4 h-4 object-contain transition-all ease-in-out group-hover:drop-shadow-[0_0_5px_color-mix(in_srgb,var(--color-accent-neon)_80%,transparent)] ${
                          smiley.isSmileyTransitioning
                            ? 'opacity-0 scale-50 duration-100'
                            : 'opacity-100 scale-100 duration-150'
                        }`}
                      />
                    )}
                  </button>
                  </Tooltip>
                  )}
                  {/* Send-as account picker, just right of the emote button */}
                  {showSendAsPicker && (
                    <div className={`absolute ${showCommandButton ? 'left-[60px]' : 'left-8'} top-1/2 -translate-y-1/2 z-10`}>
                      <SendAsPicker />
                    </div>
                  )}
                  {/* @ Mention Autocomplete */}
                  {showMentionAutocomplete && (
                    <MentionAutocomplete
                      users={getMatchingUsers(mentionQuery)}
                      selectedIndex={mentionSelectedIndex}
                      onSelect={(user) => insertMention(user)}
                      onSelectedIndexChange={setMentionSelectedIndex}
                    />
                  )}
                  {/* Emote tab completion carousel */}
                  <AnimatePresence>
                    {emoteTabState && !showMentionAutocomplete && !showCommandAutocomplete && (
                      <EmoteAutocomplete
                        current={emoteTabState.matches[emoteTabState.index]}
                        backwards={emoteTabState.matches.slice(Math.max(0, emoteTabState.index - 3), emoteTabState.index)}
                        forwards={emoteTabState.matches.slice(emoteTabState.index + 1, emoteTabState.index + 4)}
                      />
                    )}
                  </AnimatePresence>
                  {remindOverlayActive && (
                    <div
                      ref={remindBackdropRef}
                      aria-hidden="true"
                      className="glass-input pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words text-sm leading-[1.4]"
                      style={{
                        paddingTop: '8px',
                        paddingBottom: '8px',
                        paddingLeft: composerPaddingLeft,
                        paddingRight: '12px',
                        color: 'var(--color-text-primary)',
                        wordSpacing: '0.4em',
                      }}
                    >
                      {remindOverlay!.map((s, i) =>
                        s.chip ? (
                          <span key={i} style={remindChipStyle(s.kind)}>{s.text}</span>
                        ) : (
                          <span key={i}>{s.text}</span>
                        ),
                      )}
                    </div>
                  )}
                  <textarea
                    id="chat-compose-input"
                    ref={inputRef}
                    value={messageInput}
                    onChange={handleInputChange}
                    onKeyDown={handleKeyPress}
                    onPaste={(e) => void handlePaste(e)}
                    onFocus={warmSpellcheck}
                    // Ours replaces the webview's built-in checker entirely.
                    // Leaving the native one on would draw a second set of
                    // squiggles that disagrees with these (it flags every emote
                    // name), and WebView2 also pops its own spelling menu on a
                    // plain click in a misspelled word.
                    spellCheck={false}
                    data-spellcheck={spellcheckEnabled ? 'true' : undefined}
                    data-spellcheck-emotes={spellEmoteKey ?? undefined}
                    onScroll={(e) => {
                      if (remindBackdropRef.current) {
                        remindBackdropRef.current.scrollTop = e.currentTarget.scrollTop;
                        remindBackdropRef.current.scrollLeft = e.currentTarget.scrollLeft;
                      }
                      if (spellUnderlayRef.current) {
                        spellUnderlayRef.current.scrollTop = e.currentTarget.scrollTop;
                        spellUnderlayRef.current.scrollLeft = e.currentTarget.scrollLeft;
                      }
                    }}
                    placeholder={visiblePlaceholder}
                    className={`relative w-full text-sm placeholder-textSecondary resize-none overflow-hidden scrollbar-thin leading-[1.4] self-center transition-all duration-300 ${remindOverlayActive ? '' : 'glass-input'} ${
                      isWatchStreakMode 
                        ? 'ring-2 ring-amber-500/50 bg-amber-500/5 shadow-[0_0_15px_color-mix(in_srgb,var(--color-warning)_15%,transparent)] placeholder-amber-500/60 text-textPrimary'
                        : (commandState.isValid && !remindOverlayActive)
                        ? 'font-bold tracking-wide'
                        : ''
                    }`}
                    style={{ 
                      minHeight: '36px',
                      maxHeight: '120px',
                      paddingTop: '8px',
                      paddingBottom: '8px',
                      paddingLeft: composerPaddingLeft,
                      paddingRight: '12px',
                      ...(remindOverlayActive ? {
                        // The backdrop above renders the surface + chipped text;
                        // the textarea goes transparent and only carries the caret
                        // + editing. Keep a 1px transparent border so its box model
                        // matches the backdrop's bordered box and the text aligns.
                        color: 'transparent',
                        caretColor: 'var(--color-text-primary)',
                        background: 'transparent',
                        border: '1px solid transparent',
                        boxShadow: 'none',
                        backdropFilter: 'none',
                        WebkitBackdropFilter: 'none',
                        // Match the backdrop's gap so the caret stays aligned with
                        // the boxed text above it.
                        wordSpacing: '0.4em',
                        // No fade: go transparent instantly so the text doesn't
                        // ghost over the chip backdrop during the transition.
                        transition: 'none',
                      } : commandState.isValid && !isWatchStreakMode ? {
                        backgroundColor: 'rgba(200, 224, 232, 0.1)',
                        borderColor: 'var(--color-accent)',
                        boxShadow: '0 0 16px color-mix(in srgb, var(--color-accent-neon) 25%, transparent), inset 4px 4px 10px -3px rgba(0, 0, 0, 0.6)',
                        color: 'var(--color-accent)'
                      } : commandState.isCommand && !isWatchStreakMode ? {
                        backgroundColor: 'rgba(255, 255, 255, 0.04)',
                        borderColor: 'rgba(255, 255, 255, 0.2)',
                        color: 'var(--color-text-primary)'
                      } : {
                        color: 'var(--color-text-primary)'
                      })
                    }}
                    rows={1}
                    disabled={isInputDisabled}
                    onBlur={() => {
                      // Delay hiding to allow click on autocomplete
                      setTimeout(() => setShowMentionAutocomplete(false), 150);
                      setTimeout(() => setShowCommandAutocomplete(false), 150);
                      setEmoteTabState(null);
                    }}
                  />
                  {/* Length label and slow-mode countdown, bottom-right of the field. */}
                  {(messageInput.length > 0 || slowRemaining > 0) && (
                    <span className="pointer-events-none absolute bottom-1 right-2 z-10 flex items-center gap-1.5 text-[10px] tabular-nums text-textSecondary/70">
                      {slowRemaining > 0 && <span className="text-amber-300/80">slow {slowRemaining}s</span>}
                      {messageInput.length > 0 && (
                        <span className={messageInput.length > 500 ? 'text-error' : ''}>{messageInput.length}/500</span>
                      )}
                    </span>
                  )}
                  {/* Squiggles go over the textarea, not under it — the input's
                      own background would hide them. Same padding as above so
                      the mirrored text lands on the real text. */}
                  {spellRanges.length > 0 && (
                    <SpellcheckUnderlay
                      innerRef={spellUnderlayRef}
                      text={messageInput}
                      ranges={spellRanges}
                      className="text-sm leading-[1.4]"
                      style={{
                        paddingTop: '8px',
                        paddingBottom: '8px',
                        paddingLeft: composerPaddingLeft,
                        paddingRight: '12px',
                      }}
                    />
                  )}
                </div>
                <Tooltip content={isWatchStreakMode ? "Share Watch Streak" : "Send message"} side="top">
                <button
                  onClick={() => handleSendMessage()}
                  disabled={(!messageInput.trim() && !isWatchStreakMode && !isResubMode) || isInputDisabled} 
                  className={`flex-shrink-0 flex items-center justify-center self-center w-9 h-9 text-white rounded transition-all duration-300 disabled:opacity-50 disabled:cursor-not-allowed ${
                    isWatchStreakMode 
                      ? 'bg-amber-500 hover:bg-amber-400 shadow-[0_0_12px_color-mix(in_srgb,var(--color-warning)_30%,transparent)]'
                      : 'glass-button'
                  }`}
                >
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8" /></svg>
                </button>
                </Tooltip>
              </div>
            </div>
            {!isConnected && <p className="text-xs text-yellow-400 mt-2">Chat is not connected. Messages cannot be sent.</p>}
            {/* Connect chips only in the MAIN app — either watching Twitch (no
                override at all) or watching a provider stream in the main window
                (`context: 'main'`). In a MultiChat popout sign-in lives in Account
                Connections, so the chat space stays clean. */}
            {/* Both chips run the SAME connect the Accounts panel runs. This used
                to call `kick_connect` directly, which takes the OAuth token and
                nothing else — so connecting from here left you "Connected" with a
                permanently empty Following list and no way to find out why. */}
            {isMainSurface && provider === 'kick' && !kickConnected && (
              <div className="mt-2 flex justify-end">
                <PlatformAccountChip
                  connected={kickConnected}
                  onConnect={() => void connectPlatformAccount('kick')}
                />
              </div>
            )}
            {isMainSurface && provider === 'youtube' && !youtubeConnected && (
              <div className="mt-2 flex justify-end">
                <PlatformAccountChip
                  provider="youtube"
                  connected={youtubeConnected}
                  onConnect={() => void connectPlatformAccount('youtube')}
                />
              </div>
            )}
          </div>
        </div>}
      </div>
      {
        selectedUser && (
          <UserProfileCard userId={selectedUser.userId} username={selectedUser.username} displayName={selectedUser.displayName}
            color={selectedUser.color} badges={selectedUser.badges}
            messageHistory={
              userMessageHistory.current.get(
                provider === 'twitch' ? selectedUser.userId : `${provider}:${selectedUser.userId}`,
              ) || []
            }
            onClose={() => setSelectedUser(null)} position={selectedUser.position}
            isModerator={isModerator}
            viewerIsBroadcaster={!!isBroadcaster}
            broadcasterId={currentStream?.user_id}
            provider={provider}
            onPreFillCommand={preFillCommand} />
        )
      }
    </>
  );
};

export default ChatWidget;

