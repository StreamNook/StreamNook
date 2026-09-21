import React, { useMemo, useState, useEffect, useRef, memo, useSyncExternalStore, type JSX } from 'react';
import { Gift } from 'lucide-react';
import { Tooltip } from './ui/Tooltip';
import { parseMessage } from '../services/twitchChat';
import { queueEmoteForDisplayCaching, getCachedEmoteUrl, getEmoteLookup, inlineEmoteTier, sevenTvTierUrl, EmoteSet } from '../services/emoteService';
import { getCachedEmojiUrl, parseEmojisSync } from '../services/emojiService';
import { calculateHalfPadding } from '../utils/chatLayoutUtils';
import { computePaintStyle, getBadgeImageUrl, getBadgeFallbackUrls, queueCosmeticForCaching } from '../services/seventvService';
import { StyledChatName } from './chat/StyledChatName';
import { useDragModerationStore } from '../stores/dragModerationStore';
import { usePinStore } from '../stores/pinStore';
import { FallbackImage } from './FallbackImage';
import { getCosmeticsWithFallback } from '../services/cosmeticsCache';
import type { ThirdPartyBadge as ThirdPartyBadgeType } from '../services/badgeService';
import { useAppStore } from '../stores/AppStore';
import { openBadgesWithBadgeInMain } from '../utils/openBadgesInMain';
import { openExternal } from '../utils/openExternal';
import { useChatUserStore } from '../stores/chatUserStore';
import { useShallow } from 'zustand/react/shallow';
import { boundedSet } from '../utils/boundedMap';
import { useGiftBombRecipients } from '../stores/giftBombStore';
import { useMessageRepeat } from '../stores/messageRepeatStore';
import { ChannelPointsIcon } from './ChannelPointsIcon';
import { useUserColor } from '../services/userColorCache';
import { queueBadgeForCaching, getCachedBadgeUrl } from '../services/badgeImageCacheService';
import { isStreamNookUser, getStreamNookUserNumber, subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion } from '../services/supabaseService';
import { StreamNookBadge } from './StreamNookBadge';
import { AtmosphereBackground } from './AtmosphereBackground';
import { MajorCologneChrome } from './MajorCologneChrome';
import { getAtmosphere } from '../services/atmospheres';
import { features } from '../features';
import { MAJOR_COLOGNE_THEME_ID } from '../services/cologneEvent';
import type { HighlightMatch } from '../utils/chatHighlightMatcher';
import { useStreamerMode } from '../utils/streamerMode';
import { staticEmoteUrl, static7tvSrcSet } from '../utils/staticEmoteUrl';
import { flashTitle } from '../utils/titleFlasher';
import { playSoundThrottled } from '../utils/notificationSound';
import { chatterId, chatterProvider } from '../utils/chatterIdentity';
import type { ProviderId } from '../types/providers';
import { getDisplayedName, getColorOverride } from '../utils/userChatOverrides';
import { useNameColorAdjust } from '../hooks/useNameColor';
import { vendorEmojiUrl } from '../services/emojiService';
import { categoryOf, chatEventTemplateContext } from '../utils/chatEvents';
import { renderEventTemplate } from './overlay/overlayConfig';
import { convert as convertMoney, formatMoney, preloadRates, symbolToCode } from '../services/currencyService';
import { CHANNEL_SPECIFIC_TWITCH_BADGES, orderTwitchBadges } from '../utils/badgeOrder';
import { LinkPreviewCard } from './chat/LinkPreviewCard';
import { SongCard } from './chat/SongCard';
import { extractPreviewUrls, prettyUrlLabel } from '../services/linkPreviewService';
import {
  MOD_HIDDEN,
  MOD_GROW_X,
  MOD_SLIDE,
  MOD_NO_SPACE,
  MOD_PREFIX,
  MOD_BTTV_WIDE,
  MOD_BTTV_ROTATE_L,
  MOD_BTTV_ROTATE_R,
  MOD_NON_EFFECT_BITS,
  MODIFIER_TRANSFORMS,
  MODIFIER_FILTERS,
  ANIMATED_MODIFIERS,
} from '../utils/emoteModifiers';

/* eslint-disable @typescript-eslint/no-explicit-any */

// Lazy module loaders, kept at module scope on purpose: the dynamic import()
// is what keeps these modules out of the chat popout's eager graph, and the
// React Compiler cannot lower an import() expression inside a compiled
// component, so the expression lives here and the component calls a plain
// function.
const loadInvoke = () => import('@tauri-apps/api/core').then((m) => m.invoke);

// Owns the try/catch for the moderation and pin handlers below. The compiler
// cannot lower a conditional inside a try/catch in a compiled component, and
// those handlers only need "run this, report if it throws".
async function attempt(fn: () => Promise<void>, onError: (err: unknown) => void): Promise<void> {
  try {
    await fn();
  } catch (err) {
    onError(err);
  }
}
// 7TV cosmetics have complex dynamic structures that vary by API version
// Using 'any' is pragmatic here given the API complexity
type SevenTVPaintWithSelection = any;
type SevenTVBadgeWithSelection = any;

// EmoteSegment type definition (migrated from emoteParser.ts)
interface EmoteSegment {
  type: 'text' | 'emote' | 'emoji' | 'cheermote' | 'gif';
  content: string;
  emoteId?: string;
  emoteUrl?: string;
  emojiUrl?: string;
  // Cheermote-specific properties
  cheermoteUrl?: string;
  prefix?: string;
  bits?: number;
  tier?: string;
  color?: string;
  isZeroWidth?: boolean;
  /** Modifier bitmask; present only on modifier emotes (FFZ or BetterTTV) */
  modifierFlags?: number;
  /** A 7TV personal emote (the sender's own set, not the channel's). */
  isPersonal?: boolean;
  // Twitch chat GIF: the asset URL (used exactly as sent) and GIPHY id.
  gifId?: string;
  gifUrl?: string;
}

// Wrap a rendered emote group in the effect layers its aggregated modifier
// flags demand. Every wrapper is inline-block: CSS transforms are a no-op on
// plain inline elements. The outermost wrapper takes over the group's
// alignment and margin (the grid span drops them when wrapped).
//
// `pullBack` applies BetterTTV's zero-space modifier (z!), which eats the
// space before the target rather than overlaying it. It is suppressed for the
// first element of a message so the emote does not hang off the left edge.
const wrapWithEmoteEffects = (
  node: React.ReactNode,
  flags: number,
  key: string,
  pullBack: boolean,
): React.ReactNode => {
  let out = node;
  const transforms: string[] = [];
  for (const [bit, value] of MODIFIER_TRANSFORMS) {
    if (flags & bit) transforms.push(value);
  }
  if (transforms.length) {
    out = (
      <span className="inline-block" style={{ transform: transforms.join(' ') }}>
        {out}
      </span>
    );
  }
  const filters: string[] = [];
  for (const [bit, value] of MODIFIER_FILTERS) {
    if (flags & bit) filters.push(value);
  }
  if (filters.length) {
    out = (
      <span className="inline-block" style={{ filter: filters.join(' ') }}>
        {out}
      </span>
    );
  }
  for (const [bit, cls] of ANIMATED_MODIFIERS) {
    if (flags & bit) {
      out = <span className={`inline-block ${cls}`}>{out}</span>;
    }
  }
  if (flags & MOD_SLIDE) {
    // Slide is a marquee: the animated layer translates inside a clipped frame.
    out = <span className="inline-block overflow-hidden">{out}</span>;
  }
  const noSpace = (flags & MOD_NO_SPACE) !== 0 && pullBack;
  return (
    <span
      key={key}
      className={`inline-block align-middle ${noSpace ? 'mr-0.5' : 'mx-0.5'}`}
      style={noSpace ? { marginLeft: '-4px' } : undefined}
    >
      {out}
    </span>
  );
};

// Giant (gigantified) emote sizing: 4x the 2em inline height (112px at the
// default 14px chat font, matching FFZ), tracking font size and emote scale.
const GIANT_EMOTE_HEIGHT = 'calc(8em * var(--sn-emote-scale, 1))';
const GIANT_EMOTE_MAX_WIDTH = 'calc(24em * var(--sn-emote-scale, 1))';

// Best 4x-equivalent CDN URL per provider. Twitch `default` serves the
// animated variant automatically; BTTV tops out at 3x; FFZ /4 may fail for
// some emotes (onError falls back to the inline URL).
const giantEmoteCdnUrl = (url: string, provider: string | undefined, id?: string): string => {
  if (provider === 'twitch' && id)
    return `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark/4.0`;
  if (provider === '7tv' && id) return sevenTvTierUrl(id, '4x');
  if (provider === 'bttv' && id) return `https://cdn.betterttv.net/emote/${id}/3x`;
  if (provider === 'ffz') return url.replace(/\/[124]$/, '/4');
  return url;
};

// FFZ GrowX (ffzW) stretches the target emote to double width. The natural
// aspect ratio is unknowable in CSS (transform: scaleX(2) would not widen the
// grid cell, overlapping neighbors), so measure on load and set an explicit
// stretched width. Module-level so re-renders don't remount every image.
const GrowXEmoteImg = (
  props: React.ImgHTMLAttributes<HTMLImageElement> & { baseHeight: string },
) => {
  const { baseHeight, style, ...rest } = props;
  const [ratio, setRatio] = useState<number | null>(null);
  const measure = (el: HTMLImageElement | null) => {
    // The ref path covers cache-complete images where onLoad may never fire.
    if (el && el.complete && el.naturalHeight > 0) {
      setRatio(el.naturalWidth / el.naturalHeight);
    }
  };
  return (
    <img
      {...rest}
      ref={measure}
      onLoad={(e) => measure(e.currentTarget)}
      style={{
        ...style,
        height: baseHeight,
        width: ratio ? `calc(${baseHeight} * ${(2 * ratio).toFixed(4)})` : 'auto',
        maxWidth: 'calc(256px * var(--sn-emote-scale, 1))',
      }}
    />
  );
};

// BetterTTV's w! forces the emote box to a 4:1 stretch (its CSS is a literal
// 112x28 with object-fit: fill at the 28px chat size). Unlike FFZ's ffzW this
// deliberately does NOT preserve aspect ratio, so nothing needs measuring.
const WideEmoteImg = (
  props: React.ImgHTMLAttributes<HTMLImageElement> & { baseHeight: string },
) => {
  const { baseHeight, style, ...rest } = props;
  return (
    <img
      {...rest}
      style={{
        ...style,
        height: baseHeight,
        width: `calc(${baseHeight} * 4)`,
        objectFit: 'fill',
        maxWidth: 'calc(512px * var(--sn-emote-scale, 1))',
      }}
    />
  );
};

// Parse plain text into emote/emoji/text segments by name lookup against the
// loaded emote sets. Serves the local-echo fallback (no Rust segments yet) and
// the reply preview, which only ever has the parent message as raw text.
const parseTextWithEmoteSets = (text: string, emotes?: EmoteSet | null): EmoteSegment[] => {
  const words = text.split(' ');
  const segments: EmoteSegment[] = [];

  words.forEach((word, i) => {
    if (i > 0) segments.push({ type: 'text', content: ' ' });

    // Per-set name index (7tv > bttv > ffz > twitch > kick > youtube,
    // first-wins - same order the old find() chain searched). Kick/YouTube
    // matter here because a Kick reply preview arrives as plain text with its
    // emote tokens already reduced to names. This runs per word of every reply
    // preview, so the old six-array linear scan was O(words x set size).
    const emote = emotes ? getEmoteLookup(emotes).byName.get(word) : undefined;

    if (emote) {
      segments.push({
        type: 'emote',
        content: emote.name,
        emoteId: emote.id,
        emoteUrl: emote.url,
        isZeroWidth: emote.isZeroWidth || (emote as any).is_zero_width,
        modifierFlags: emote.modifierFlags ?? (emote as any).modifier_flags,
      });
    } else {
      // Parse the word for emojis for iOS-style emoji rendering
      parseEmojisSync(word).forEach((seg) => {
        if (seg.type === 'emoji' && seg.emojiUrl) {
          segments.push({ type: 'emoji', content: seg.content, emojiUrl: seg.emojiUrl });
        } else {
          segments.push({ type: 'text', content: seg.content });
        }
      });
    }
  });

  // Coalesce adjacent text segments
  const coalesced: EmoteSegment[] = [];
  segments.forEach((seg) => {
    if (coalesced.length > 0 && coalesced[coalesced.length - 1].type === 'text' && seg.type === 'text') {
      coalesced[coalesced.length - 1].content += seg.content;
    } else {
      coalesced.push(seg);
    }
  });
  return coalesced;
};

// Global cache for channel names and profile images to prevent re-fetching and flashing
const channelNameCache = new Map<string, string>();
const channelProfileImageCache = new Map<string, string>();

/**
 * Get the best URL for a Twitch badge, with reactive caching.
 * - Channel-specific badges (subscriber, bits, etc.) are never cached to avoid cross-channel pollution
 * - Global badges are cached locally for faster loading
 */
function getTwitchBadgeUrl(badgeKey: string, badgeInfo: { localUrl?: string; url?: string; image_url_4x?: string; image_url_2x?: string; image_url_1x?: string }): string {
  // If already has a local URL, use it
  if (badgeInfo.localUrl) {
    return badgeInfo.localUrl;
  }

  // Extract set ID from badge key (format: "set/version")
  const setId = badgeKey.split('/')[0];
  
  // Channel-specific badges are never cached - always use remote URL
  // This prevents cross-channel pollution where one streamer's sub badge appears in another's chat
  if (CHANNEL_SPECIFIC_TWITCH_BADGES.has(setId)) {
    return badgeInfo.image_url_4x || badgeInfo.image_url_1x || '';
  }

  // Global badges can be cached safely
  const cacheId = `twitch-${badgeKey}`;
  
  // Check if we have a cached version
  const cachedUrl = getCachedBadgeUrl(cacheId);
  if (cachedUrl) {
    return cachedUrl;
  }

  // Not cached - queue for caching and return remote 4x URL
  const remoteUrl = badgeInfo.image_url_4x;
  if (remoteUrl) {
    queueBadgeForCaching(cacheId, remoteUrl);
  }

  return remoteUrl || badgeInfo.image_url_1x || '';
}

import { BackendChatMessage } from '../services/twitchChat';

import { Logger } from '../utils/logger';
import { kickTimeoutMinutes } from '../utils/kickTimeout';

interface ChatMessageProps {
  message: string | BackendChatMessage; // Raw IRC message or Backend Message Object
  onUsernameClick?: (
    userId: string,
    username: string,
    displayName: string,
    color: string,
    badges: Array<{ key: string; info: { url?: string; image_url_4x?: string } }>,

    event: React.MouseEvent,
    /** Which platform `userId` belongs to. Absent means Twitch, so every existing
     *  caller is unchanged. Without it the card would look a Kick id up on Helix. */
    provider?: ProviderId,
  ) => void;
  onReplyClick?: (parentMsgId: string) => void;
  isHighlighted?: boolean;
  moderationContext?: { type: 'timeout' | 'ban' | 'deleted'; duration?: number } | null; // Moderation context from CLEARMSG/CLEARCHAT
  onEmoteRightClick?: (emoteName: string) => void;
  onMessageCopy?: (content: string) => void;
  onUsernameRightClick?: (messageId: string, username: string) => void;
  onBadgeClick?: (badgeKey: string, badgeInfo: { url?: string; image_url_4x?: string }) => void;
  emotes?: EmoteSet | null;
  isModerator?: boolean;
  broadcasterId?: string;
}

/**
 * Component for rendering @mentions with user's 7TV paint styling
 */
// Resolution cache for @mentions of users NOT in the chat store. Without it,
// every rendered mention of an absent user fired its own uncached Helix
// round-trip per row (a wall of "@someuser" spam = one request per message).
// Rejections are cached too (unknown/misspelled names), on a shorter TTL.
type MentionUser = { id: string; login: string; display_name: string } | null;
const mentionUserCache = new Map<string, { user: MentionUser; ts: number }>();
const mentionUserInflight = new Map<string, Promise<MentionUser>>();
const MENTION_POS_TTL_MS = 10 * 60_000;
const MENTION_NEG_TTL_MS = 60_000;
const MENTION_CACHE_MAX = 500;

function resolveMentionUser(username: string): Promise<MentionUser> {
  const key = username.toLowerCase();
  const hit = mentionUserCache.get(key);
  if (hit && Date.now() - hit.ts < (hit.user ? MENTION_POS_TTL_MS : MENTION_NEG_TTL_MS)) {
    return Promise.resolve(hit.user);
  }
  const inflight = mentionUserInflight.get(key);
  if (inflight) return inflight;
  const p = import('@tauri-apps/api/core')
    .then(({ invoke }) =>
      invoke<{ id: string; login: string; display_name: string }>('get_user_by_login', { login: key }),
    )
    .catch(() => null as MentionUser)
    .then((user) => {
      boundedSet(mentionUserCache, key, { user, ts: Date.now() }, MENTION_CACHE_MAX);
      return user;
    })
    .finally(() => {
      mentionUserInflight.delete(key);
    });
  mentionUserInflight.set(key, p);
  return p;
}

const MentionSpan: React.FC<{
  username: string;
  onUsernameClick?: ChatMessageProps['onUsernameClick'];
}> = ({ username, onUsernameClick }) => {
  // Subscribe to the specific user from the store (reactive updates)
  const cachedUser = useChatUserStore((state) => state.getUserByUsername(username));
  
  // Local state for users not in chat store (fallback API lookup)
  const [apiUserPaint, setApiUserPaint] = useState<SevenTVPaintWithSelection | null>(null);
  
  // If user is in chat store, use their data directly
  const adjustMentionColor = useNameColorAdjust();
  const userColor = adjustMentionColor(cachedUser?.color || '#9147FF') ?? '#9147FF';
  const userPaint = cachedUser?.paint || apiUserPaint;
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';
  
  // Only do API lookup if user is NOT in the chat store. resolveMentionUser
  // dedupes and caches (including not-found), so repeated mentions of the same
  // absent user across many rows cost one request total.
  useEffect(() => {
    if (cachedUser) return; // Already have data from store

    let isMounted = true;

    resolveMentionUser(username)
      .then((user) => {
        if (!isMounted || !user) return;
        // Try to get cosmetics for this user (includes paint); this path has
        // its own in-flight dedupe + LRU in cosmeticsCache.
        getCosmeticsWithFallback(user.id)
          .then((cosmetics) => {
            if (!isMounted || !cosmetics) return;
            const selectedPaint = cosmetics.paints?.find(
              (p: SevenTVPaintWithSelection) => p.selected,
            );
            if (selectedPaint) {
              setApiUserPaint(selectedPaint);
            }
          })
          .catch(() => {});
      })
      .catch(() => {});

    return () => {
      isMounted = false;
    };
  }, [username, cachedUser]);
  
  // Compute paint style - use user's Twitch color as fallback (not accent).
  // When the user has turned off paint-on-mentions globally, fall back to the
  // flat color even if the user has a paint set.
  const paintMentionsInBody = useAppStore((s) => s.settings.chat_design?.paint_mentions_in_body) ?? true;
  const nameStyle = useMemo(() => {
    if (userPaint && paintMentionsInBody) {
      return computePaintStyle(userPaint, userColor, paintShadowMode);
    }
    // Use user's Twitch chat color as fallback
    return { color: userColor };
  }, [userPaint, userColor, paintShadowMode, paintMentionsInBody]);
  
  const handleClick = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!onUsernameClick) return;
    
    // Use cached user data if available
    if (cachedUser) {
      onUsernameClick(
        cachedUser.userId,
        cachedUser.username,
        cachedUser.displayName,
        cachedUser.color,
        [],
        e
      );
      return;
    }
    
    // Fallback to API lookup (shared cache with the mount effect)
    try {
      const user = await resolveMentionUser(username);
      if (user) {
        onUsernameClick(
          user.id,
          user.login,
          user.display_name,
          userColor,
          [],
          e
        );
      }
    } catch (err) {
      Logger.warn('[MentionSpan] Could not look up mentioned user:', err);
    }
  };
  
  // No pill fill here: normal-chat mentions are plain colored names. A latent
  // bg-accent/15 never compiled until the 2026-07-26 token-alpha fix, then
  // surfaced as an accidental pill and was removed.
  return (
    <Tooltip content={`View ${username}'s profile`} side="top">
      <span
        className="inline-block px-1.5 py-0.5 rounded font-medium cursor-pointer transition-colors"
        style={nameStyle}
        onClick={handleClick}
      >
        @{username}
      </span>
    </Tooltip>
  );
};

// Custom comparison function for React.memo
// Only re-render if the message content or highlight state actually changes
const chatMessageAreEqual = (prevProps: ChatMessageProps, nextProps: ChatMessageProps): boolean => {
  // Only re-render if these props actually changed
  if (prevProps.message !== nextProps.message) {
    // If objects, compare IDs
    if (typeof prevProps.message !== 'string' && typeof nextProps.message !== 'string') {
      if (prevProps.message.id !== nextProps.message.id) return false;
    } else {
      return false;
    }
  }
  // messageIndex is intentionally NOT compared. It is the array position, which
  // shifts by N on every flush once the list sits at its cap (busy channel), so
  // comparing it re-rendered EVERY visible row on EVERY flush — the main-thread
  // hog behind choppy hype-train/confetti animations. Zebra striping now comes
  // from CSS :nth-child, so the row no longer needs the index at all.
  if (prevProps.isHighlighted !== nextProps.isHighlighted) return false;
  if (prevProps.moderationContext?.type !== nextProps.moderationContext?.type ||
      prevProps.moderationContext?.duration !== nextProps.moderationContext?.duration) return false;
  // The emote set DOES affect output: a row rendered before the channel's set
  // resolved shows emote codes as plain text and would otherwise never repaint.
  // Safe to compare by identity and cheap to include — useChannelEmotes returns
  // a stable reference from the module-level emoteCache, so this only trips when
  // the set is genuinely replaced (first load, /refresh, 7TV EventAPI update).
  if (prevProps.emotes !== nextProps.emotes) return false;

  // All other props (callbacks) can be ignored for re-render decisions
  // since they don't affect the visual output of the message
  return true;
};

// Stable empty reference for the third-party badge read below, so the zustand
// selector returns the same array identity for every non-member chatter (a
// fresh [] each render would re-render every row on any store change).
const EMPTY_THIRD_PARTY: ThirdPartyBadgeType[] = [];
const EMPTY_HIDDEN_PROVIDERS: string[] = [];
const EMPTY_TWITCH_BADGES: never[] = [];

// Memoized ChatMessage component to prevent unnecessary re-renders
// This is critical for preventing animation restarts when new messages arrive
// A username rendered with its own cosmetics (gift recipients, sub-message
// names). Module-level so it is a stable component with its own hooks; it
// used to be declared inside ChatMessage's render, which remounted it on
// every render and put hook calls inside a nested function.
function UsernameWithCosmetics({
  username,
  userIdProp,
  displayName,
  userOverrides,
  paintShadowMode,
  onUsernameClick,
}: {
  username: string;
  userIdProp: string | null;
  displayName?: string;
  userOverrides: Parameters<typeof getDisplayedName>[2];
  paintShadowMode: Parameters<typeof computePaintStyle>[2];
  onUsernameClick: ChatMessageProps['onUsernameClick'];
}) {
  const [userCosmetics, setUserCosmetics] = useState<{ badges: any[]; paints: any[] } | null>(null);
  const [userBadges] = useState<Array<{ key: string; info: any }>>([]);

  useEffect(() => {
    if (!userIdProp) return;

    let cancelled = false;

    // Fetch 7TV cosmetics (with cache fallback)
    getCosmeticsWithFallback(userIdProp).then((cosmetics) => {
      if (cancelled || !cosmetics) return;
      setUserCosmetics(cosmetics);
    });

    // Fetch Twitch badges - we don't have badge string from recipient, so skip for now
    // Recipients will just show their 7TV cosmetics

    return () => {
      cancelled = true;
    };
  }, [userIdProp]);

  const userPaint = userCosmetics?.paints.find((p) => p.selected);
  const userBadge = userCosmetics?.badges.find((b) => b.selected);

  // Resolve a per-recipient color: a manual override wins, else the user's
  // real Twitch name color (batched Helix lookup), else Twitch purple. This
  // is why a gift recipient no longer inherits the gifter's color.
  const fetchedColor = useUserColor(userIdProp);
  const adjustRecipientColor = useNameColorAdjust();
  const recipientBaseColor =
    adjustRecipientColor(getColorOverride(userIdProp, userOverrides) ?? fetchedColor ?? '#9147FF') ?? '#9147FF';

  const userStyle = useMemo(() => {
    if (!userPaint) {
      return { color: recipientBaseColor };
    }
    return computePaintStyle(userPaint, recipientBaseColor, paintShadowMode);
  }, [userPaint, recipientBaseColor, paintShadowMode]);

  return (
    <span className="inline-flex items-center align-middle">
      {userBadge && (
        <span className="inline-flex items-center align-middle gap-1 mr-1">
          <Tooltip content={userBadge.description || userBadge.name} side="top">
            <img
              src={getBadgeImageUrl(userBadge)}
              alt={userBadge.description || userBadge.name}
              className="sn-chat-badge inline-block"
              onError={(e) => {
                e.currentTarget.style.display = 'none';
              }}
            />
          </Tooltip>
        </span>
      )}
      <Tooltip content="Click to view profile" side="top">
        <span
          className="font-bold cursor-pointer hover:underline"
          style={userStyle}
          onClick={(e) => {
            if (userIdProp && onUsernameClick) {
              onUsernameClick(
                userIdProp,
                username,
                displayName || username,
                userStyle.color as string || '#9147FF',
                userBadges,
                e
              );
            }
          }}
        >
          {getDisplayedName(userIdProp, displayName || username, userOverrides)}
        </span>
      </Tooltip>
    </span>
  );
}

/** Timeout durations for the hover dock: settings.moderation.timeout_presets
 *  (seconds) or the classic 1s / 10m / 1h / 24h. */
function formatTimeoutLabel(seconds: number): string {
  if (seconds % 604800 === 0 && seconds >= 604800) return `${seconds / 604800}w`;
  if (seconds % 86400 === 0 && seconds >= 86400) return `${seconds / 86400}d`;
  if (seconds % 3600 === 0 && seconds >= 3600) return `${seconds / 3600}h`;
  if (seconds % 60 === 0 && seconds >= 60) return `${seconds / 60}m`;
  return `${seconds}s`;
}
const DEFAULT_TIMEOUT_PRESETS = [1, 600, 3600, 86400];
function presetsFromSetting(raw: number[] | undefined): Array<{ label: string; val: number }> {
  const list = (raw && raw.length > 0 ? raw : DEFAULT_TIMEOUT_PRESETS)
    .filter((n) => Number.isFinite(n) && n >= 1 && n <= 1209600)
    .slice(0, 8);
  return (list.length > 0 ? list : DEFAULT_TIMEOUT_PRESETS).map((val) => ({ label: formatTimeoutLabel(val), val }));
}

const ChatMessage = memo(function ChatMessageInner({ message, onUsernameClick, onReplyClick, isHighlighted = false, moderationContext = null, onEmoteRightClick, onMessageCopy, onUsernameRightClick, onBadgeClick, emotes, isModerator = false, broadcasterId }: ChatMessageProps) {
  // Field selectors, NOT a whole-store subscription. This component is mounted
  // once per chat row, so subscribing to the entire store made every row
  // re-render on every unrelated store tick (hours-watched, viewer count, etc.).
  // On a high-traffic channel that render storm pegs the main thread and freezes
  // the video. Selecting only what we read means a row re-renders only when its
  // own inputs change.
  // Narrow subscriptions: rows re-render only when a CHAT-relevant settings
  // slice changes, not on every settings write (player quality, sidebar, etc.
  // previously re-rendered every mounted row - memo does not gate a
  // component's own hook-driven updates).
  const chatDesign = useAppStore((s) => s.settings.chat_design);
  const chatCustomization = useAppStore((s) => s.settings.chat_customization);
  const chatHighlights = useAppStore((s) => s.settings.chat_highlights);
  const chatEvents = useAppStore((s) => s.settings.chat_events);
  const timeoutPresetSetting = useAppStore((s) => s.settings.moderation?.timeout_presets);
  const timeoutPresets = useMemo(
    () => presetsFromSetting(timeoutPresetSetting),
    [timeoutPresetSetting],
  );
  const currentUser = useAppStore((s) => s.currentUser);
  // Whole-message pickup attaches transient window listeners; this ref holds the
  // current teardown so an unmount can run it if the row is removed mid-press
  // (a virtualized chat can unmount a held row before any terminal pointer event
  // fires, which would otherwise leak the listeners and this row's closure).
  const pickupTeardownRef = useRef<(() => void) | null>(null);
  useEffect(() => () => pickupTeardownRef.current?.(), []);
  // Re-render this row when the StreamNook registry updates (async load / new signup)
  useSyncExternalStore(subscribeStreamNookRegistryVersion, getStreamNookRegistryVersion, getStreamNookRegistryVersion);
  const parsed = useMemo(() => {
    // Extract channel ID from the message tags if available
    // For shared chat messages, use source-room-id instead of room-id for badge lookup
    let sourceRoomId: string | undefined;
    let roomId: string | undefined;

    if (typeof message === 'string') {
      const sourceRoomIdMatch = message.match(/source-room-id=([^;]+)/);
      const roomIdMatch = message.match(/room-id=([^;]+)/);
      sourceRoomId = sourceRoomIdMatch ? sourceRoomIdMatch[1] : undefined;
      roomId = roomIdMatch ? roomIdMatch[1] : undefined;
    } else {
      // Backend message object - extract room-id from tags
      sourceRoomId = message.tags?.['source-room-id'];
      roomId = message.tags?.['room-id'];
    }

    // Prefer source-room-id for shared chat messages, fall back to room-id
    const channelId = sourceRoomId || roomId;

    return parseMessage(message, channelId);
  }, [message]);

  const isAction = parsed.isAction || false;

  // Gift-bomb recipients: when the collapse setting funnels a submysterygift's
  // subgift children into giftBombStore, this announcement card lists them.
  // Called unconditionally (rules of hooks); returns a stable empty result for
  // normal messages, where giftOriginId is undefined.
  const giftOriginId =
    parsed.tags.get('msg-param-origin-id') || parsed.tags.get('msg-param-community-gift-id') || undefined;
  const { expected: giftExpected, recipients: giftRecipients } = useGiftBombRecipients(giftOriginId);
  const [giftRecipientsExpanded, setGiftRecipientsExpanded] = useState(false);

  // Repeat run: how many copies of this message the ingest path folded into it.
  // Same shape as the gift bomb above — subscribing here, inside the row, is
  // what lets an already-mounted message update its count without new props.
  const repeat = useMessageRepeat(typeof message === 'string' ? undefined : message.id);
  const repeatSettings = useAppStore((s) => s.settings.message_repeat);
  const repeatThreshold = Math.max(2, repeatSettings?.threshold ?? 2);
  const showRepeatCount = repeat.count >= repeatThreshold;
  const repeatTooltip = useMemo(() => {
    if (!showRepeatCount) return '';
    const names = repeat.participants.map((p) => p.displayName).filter(Boolean);
    if (names.length === 0) return `Sent ${repeat.count} times`;
    const shown = names.slice(0, 10).join(', ');
    const hidden = repeat.count - 1 - Math.min(names.length, 10);
    return hidden > 0 ? `Also sent by ${shown} and ${hidden} more` : `Also sent by ${shown}`;
  }, [showRepeatCount, repeat.count, repeat.participants]);

  // PHASE 3.1 - THE ENDGAME: Use pre-formatted timestamps from Rust
  // Zero Date parsing on main thread!
  const formattedTimestamp = (() => {
    if (!chatDesign?.show_timestamps) return null;

    // Use pre-computed timestamps from Rust metadata
    if (parsed.metadata) {
      return chatDesign?.show_timestamp_seconds
        ? parsed.metadata.formatted_timestamp_with_seconds
        : parsed.metadata.formatted_timestamp;
    }

    // Fallback: compute locally if metadata not available (legacy messages)
    const tmiSentTs = parsed.tags.get('tmi-sent-ts');
    if (!tmiSentTs) return null;

    // Read the setting outside the try: the compiler cannot lower an optional
    // chain inside a try/catch, and the value does not depend on the parse.
    const withSeconds = !!chatDesign?.show_timestamp_seconds;
    const locale = navigator.language || undefined;
    try {
      const date = new Date(parseInt(tmiSentTs, 10));
      const options: Intl.DateTimeFormatOptions = {
        hour: 'numeric',
        minute: '2-digit',
      };
      if (withSeconds) {
        options.second = '2-digit';
      }
      return date.toLocaleTimeString(locale, options);
    } catch {
      return null;
    }
  })();

  // Computed in-render via useMemo rather than via useEffect+useState — the
  // previous useEffect pattern meant the FIRST render of a new message
  // committed with an empty content array; only the second render (after
  // useEffect fired) painted the actual text. In a fast burst that gap is
  // visible: the screen flashes a wall of usernames with no message bodies,
  // then the bodies pop in. Computing in-render closes the gap entirely —
  // first render contains the real content.
  const contentWithEmotes = useMemo<EmoteSegment[]>(() => {
    // Pre-parsed segments from Rust (the primary path)
    if (parsed.segments && parsed.segments.length > 0) {
      // Twitch text is already emoji-tokenized in Rust; the other providers
      // (Kick/YouTube/TikTok) emit raw text, so split their unicode emoji into
      // Apple-emoji image segments here for parity (also covers the blended feed,
      // which renders these segments directly).
      const isProvider = !!parsed.provider && parsed.provider !== 'twitch';
      return parsed.segments.flatMap((seg): EmoteSegment[] => {
        if (seg.type === 'emote') {
          return [{
            type: 'emote' as const,
            content: seg.content,
            emoteId: seg.emote_id,
            emoteUrl: seg.emote_url,
            isZeroWidth: seg.is_zero_width,
            modifierFlags: seg.modifier_flags,
            isPersonal: seg.is_personal,
          }];
        } else if (seg.type === 'emoji') {
          return [{
            type: 'emoji' as const,
            content: seg.content,
            emojiUrl: seg.emoji_url,
          }];
        } else if (seg.type === 'link') {
          // Links are handled in parseTextWithLinks
          return [{
            type: 'text' as const,
            content: seg.content,
          }];
        } else if (seg.type === 'cheermote') {
          // Cheermote segment with animated GIF and bits amount
          return [{
            type: 'cheermote' as const,
            content: seg.content,
            cheermoteUrl: seg.cheermote_url,
            prefix: seg.prefix,
            bits: seg.bits,
            tier: seg.tier,
            color: seg.color,
          }];
        } else if (seg.type === 'gif') {
          return [{
            type: 'gif' as const,
            content: seg.content,
            gifId: seg.gif_id,
            gifUrl: seg.gif_url,
          }];
        } else if (isProvider) {
          return parseEmojisSync(seg.content).map((es): EmoteSegment =>
            es.type === 'emoji' && es.emojiUrl
              ? { type: 'emoji' as const, content: es.content, emojiUrl: es.emojiUrl }
              : { type: 'text' as const, content: es.content },
          );
        } else {
          return [{
            type: 'text' as const,
            content: seg.content,
          }];
        }
      });
    }

    // Fallback for local messages (no segments from Rust yet): parse text by
    // name lookup against the loaded sets (emojis parsed either way).
    return parseTextWithEmoteSets(parsed.content, emotes);
  }, [parsed.segments, parsed.content, parsed.provider, emotes]);

  // Reply preview: the parent message only exists as raw tag text, so emotes
  // in it are resolved by name against the same loaded sets.
  const replyPreviewSegments = useMemo<EmoteSegment[] | null>(() => {
    const body = parsed.replyInfo?.parentMsgBody;
    if (!body) return null;
    return parseTextWithEmoteSets(body, emotes);
  }, [parsed.replyInfo?.parentMsgBody, emotes]);

  // Extract userId once to prevent re-renders
  const userId = useMemo(() => parsed.tags.get('user-id'), [parsed.tags]);

  // The id to open a PROFILE with, which is not the same thing on every platform.
  // Twitch carries it as the `user-id` IRC tag; Kick and YouTube send no such tag
  // and put the chatter's id on `providerUserId`. Every click site used to read
  // the tag and bail when it was missing, so clicking a Kick or YouTube chatter
  // did nothing at all — not the wrong card, no card.
  const clickProvider = chatterProvider(parsed as never);
  const clickUserId = useMemo(
    () => chatterId(parsed as never),
    // `parsed` is rebuilt per message, so this is stable per row.
    [parsed],
  );

  // 7TV cosmetics key. Cosmetics are a 7TV-account property that syncs across all
  // of a user's linked platforms, but we resolve them per chatter by platform id.
  // Non-Twitch ids are namespaced (`kick:123`) so a Twitch id and a Kick id of the
  // same number never collide in the shared cosmetics store. Twitch keeps the bare
  // user-id tag, so its lookup is byte-identical. (Kept separate from `userId` so
  // non-cosmetic, userId-gated features like profile-card clicks stay unchanged.)
  const providerUserId = (parsed as { providerUserId?: string }).providerUserId;
  const cosmeticsKey =
    parsed.provider && parsed.provider !== 'twitch'
      ? providerUserId
        ? `${parsed.provider}:${providerUserId}`
        : undefined
      : userId;

  // Provider-aware ban/timeout. Twitch -> Helix `ban_user` (duration in SECONDS,
  // null = permanent ban). Kick -> `kick_ban_user` (duration in MINUTES, null =
  // permanent) addressed by numeric Kick ids (the broadcaster + the chatter).
  // Shift-click hands the command to the composer instead of firing it, so the
  // reason can be edited first — the same prefill-then-edit flow the user card
  // uses. Sent as a window event rather than a new prop because the message
  // list's props are performance-sensitive, and it matches how /clearmessages
  // and /usercard already talk to ChatWidget.
  const prefillModCommand = (durationSeconds: number | null) => {
    const saved = useAppStore.getState().settings.moderation?.saved_ban_reasons?.[0]?.trim() ?? '';
    const target = parsed.username;
    const cmd =
      durationSeconds == null
        ? `/ban ${target} ${saved}`
        : `/timeout ${target} ${durationSeconds} ${saved}`;
    // Scoped to this message's channel: several chat surfaces can be mounted in
    // one window (MultiNook, split panes), and an unscoped event would overwrite
    // the composer in all of them.
    window.dispatchEvent(
      new CustomEvent('streamnook-prefill-command', {
        detail: { channel: parsed.channel, command: cmd },
      }),
    );
  };

  const modBan = async (durationSeconds: number | null, reasonOverride?: string) => {
    // Falls back to the first saved reason, so the common case needs no typing.
    const saved = useAppStore.getState().settings.moderation?.saved_ban_reasons?.[0]?.trim();
    const reason = (reasonOverride ?? saved ?? '').trim() || null;
    await attempt(async () => {
      const invoke = await loadInvoke();
      if (parsed.provider === 'kick') {
        if (!providerUserId || !broadcasterId) return;
        const durationMinutes =
          durationSeconds == null ? null : kickTimeoutMinutes(durationSeconds);
        await invoke('kick_ban_user', {
          broadcasterUserId: Number(broadcasterId),
          targetUserId: Number(providerUserId),
          durationMinutes,
          reason,
        });
      } else if (parsed.provider === 'youtube') {
        // YouTube: address the chatter by their channel id; a duration = timeout
        // (YouTube's fixed length), null = permanent hide/ban.
        if (!providerUserId) return;
        await invoke('youtube_ban_user', {
          channel: parsed.channel.replace(/^youtube:/, ''),
          targetChannelId: providerUserId,
          durationSeconds,
        });
      } else {
        const targetUserId = parsed.tags.get('user-id');
        if (!targetUserId) return;
        await invoke('ban_user', { broadcasterId, targetUserId, duration: durationSeconds, reason });
      }
    }, (err) => {
      Logger.error('[ChatMessage] mod ban/timeout failed:', err);
    });
  };

  // Provider-aware delete. Twitch -> Helix `delete_chat_message`; Kick ->
  // `kick_delete_message` (DELETE /public/v1/chat/{id}).
  const modDelete = async (messageId: string) => {
    await attempt(async () => {
      const invoke = await loadInvoke();
      if (parsed.provider === 'kick') {
        await invoke('kick_delete_message', { messageId });
      } else if (parsed.provider === 'youtube') {
        await invoke('youtube_delete_message', {
          channel: parsed.channel.replace(/^youtube:/, ''),
          messageId,
        });
      } else {
        await invoke('delete_chat_message', { broadcasterId, messageId });
      }
    }, (err) => {
      Logger.error('[ChatMessage] delete failed:', err);
    });
  };

  // Link preview cards. Allowlisted hosts (YouTube, Twitch, imgur, etc.) auto-
  // expand; every other link is surfaced too but as a click-to-load chip, so an
  // arbitrary pasted URL is never fetched passively (that would reveal the
  // user's IP to the linked host). Capped per message so a link wall can't spawn
  // a card wall. Computed unconditionally here (before the event-type early
  // returns below) to keep hook order stable; only the regular-message render
  // path actually mounts the cards.
  const linkPreviewsEnabled = chatDesign?.link_previews ?? true;
  // "Clean" mode (default) hides the inline link and shows only the card. When
  // the user opts to keep the link, both the inline link and the card show.
  const keepInlineLink = chatDesign?.link_preview_keep_link ?? false;
  // Previewable URLs (≤2 trusted + ≤2 untrusted), each flagged with trust. Kept
  // as small arrays, not Sets: `.includes` over a handful of items is faster
  // than allocating a Set per message on the chat hot path.
  // Hosts the user has explicitly opted to trust, on top of the built-in
  // allowlist. Passed into extraction so those links auto-expand too.
  const userTrustedDomains = chatDesign?.link_preview_trusted_domains;
  const linkPreviewItems = useMemo(
    () => (linkPreviewsEnabled ? extractPreviewUrls(parsed.content, 2, userTrustedDomains) : []),
    [linkPreviewsEnabled, parsed.content, userTrustedDomains],
  );
  // Only trusted links suppress their inline URL in clean mode (their card is
  // the link). Untrusted links keep the inline link and add a load-on-click chip.
  const trustedPreviewUrls = useMemo(
    () => linkPreviewItems.filter((i) => i.trusted).map((i) => i.url),
    [linkPreviewItems],
  );

  // Per-user nickname/color overrides. The original display name + username
  // stay in the click payload (so the profile card still opens to the real
  // person) and in the IRC mention insertion path (Twitch only resolves real
  // @logins, never nicknames). Render path swaps in the nickname.
  const userOverrides = chatCustomization?.user_overrides;
  const originalDisplayName = useMemo(
    () => parsed.tags.get('display-name') || parsed.displayName || parsed.username,
    [parsed.tags, parsed.displayName, parsed.username],
  );
  const effectiveDisplayName = useMemo(
    () => getDisplayedName(cosmeticsKey ?? userId, originalDisplayName, userOverrides),
    [cosmeticsKey, userId, originalDisplayName, userOverrides],
  );
  // The author name shown on a normal message. Twitch keeps its exact existing
  // value (its `username` already holds the cased display name), so Twitch
  // rendering is unchanged. Non-Twitch providers (Kick) put the lowercase slug
  // in `username`, so render their cased display name instead.
  const showAtSign = chatDesign?.show_at_sign === true;
  const renderedName = useMemo(
    () => {
      const base =
        parsed.provider && parsed.provider !== 'twitch'
          ? parsed.tags.get('display-name') || parsed.displayName || parsed.username
          : parsed.username;
      return showAtSign && base && !base.startsWith('@') ? `@${base}` : base;
    },
    [parsed.provider, parsed.tags, parsed.displayName, parsed.username, showAtSign],
  );
  // Color override layers under the user's 7TV paint when one is selected
  // (the paint computes against this base color), or replaces parsed.color
  // outright when no paint is in play.
  // How a reply names its parent: the context line above the body (default),
  // an @name at the start of the body, or nothing at all.
  const replyStyle = chatDesign?.reply_style ?? 'full';
  const replyMentionNode =
    replyStyle === 'mention' && parsed.replyInfo ? (
      <span className="font-semibold opacity-85">
        @{getDisplayedName(parsed.replyInfo.parentUserId, parsed.replyInfo.parentDisplayName, userOverrides)}{' '}
      </span>
    ) : null;
  const giantJustify =
    chatDesign?.giant_emote_align === 'left'
      ? 'justify-start'
      : chatDesign?.giant_emote_align === 'right'
        ? 'justify-end'
        : 'justify-center';
  // Readability last: an override is still a color someone chose, and a
  // navy override is as hard to read as a navy Twitch color.
  const adjustNameColor = useNameColorAdjust();
  const effectiveColor = useMemo(
    () => adjustNameColor(getColorOverride(cosmeticsKey ?? userId, userOverrides) ?? parsed.color),
    [cosmeticsKey, userId, userOverrides, parsed.color, adjustNameColor],
  );

  // Paint + 7TV badge are now derived from chatUserStore: ChatWidget's addUser
  // fires the cosmetics fetch once per user and stores the selected paint+badge.
  // Subscribing here means a 100-message scrollback renders 0 paint-fetches +
  // 0 paint-derivations per message — the store holds one resolved entry per
  // unique chatter and every message just reads from it.
  //
  // ONE shallow-compared subscription for all five cosmetic fields. Five
  // separate subscriptions meant 5 store listeners per row and 5 selector
  // evaluations per store write across every mounted row; one tuple selector
  // has identical reactivity (shallow compare on the five fields matches the
  // per-field Object.is checks) at a fifth of the cost.
  const {
    paint: seventvPaintRaw,
    seventvBadge: seventvBadgeRaw,
    thirdPartyBadges: thirdPartyBadgesAll,
    atmosphereId,
    cologne,
  } = useChatUserStore(
    useShallow((s) => {
      const u = cosmeticsKey ? s.users.get(cosmeticsKey) : undefined;
      return {
        paint: u?.paint,
        seventvBadge: u?.seventvBadge,
        // A StreamNook member's curated third-party badges (BTTV / FFZ /
        // Chatterino / Homies / Chatsen / Chatty / DankChat), resolved ONCE per
        // unique member via the Identity API — never a network call in this
        // per-message hot path. Empty for non-members and for members who
        // haven't opted any badge into their loadout.
        thirdPartyBadges: u?.thirdPartyBadges ?? EMPTY_THIRD_PARTY,
        // The member's StreamNook Atmosphere -> the SAME animated wash as their
        // profile backdrop, rendered behind their message.
        atmosphereId: u?.atmosphereId ?? null,
        // CS2 Major Cologne event cosmetics (null = none). Takes precedence
        // over the Atmosphere wash when present.
        cologne: u?.cologne ?? null,
      };
    }),
  );
  const seventvPaint = seventvPaintRaw as SevenTVPaintWithSelection | null | undefined;
  // Badge controls. Filtered HERE, at the source, so the five badge render
  // sites below all follow without each knowing why.
  const badgesOn = chatDesign?.show_badges !== false;
  const thirdPartyBadgesOn = chatDesign?.show_third_party_badges !== false;
  const hiddenBadgeProviders = chatDesign?.hidden_badge_providers ?? EMPTY_HIDDEN_PROVIDERS;
  const badgeSourceHidden = (src?: string) => hiddenBadgeProviders.includes((src || '').toLowerCase());
  const visibleBadges = badgesOn ? parsed.badges : EMPTY_TWITCH_BADGES;
  const seventvBadgeAll = seventvBadgeRaw as SevenTVBadgeWithSelection | null | undefined;
  const seventvBadge = thirdPartyBadgesOn && !badgeSourceHidden('7tv') ? seventvBadgeAll : null;
  const thirdPartyBadges = useMemo(
    () => (thirdPartyBadgesOn ? thirdPartyBadgesAll.filter((b) => !badgeSourceHidden(b.provider)) : EMPTY_THIRD_PARTY),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [thirdPartyBadgesAll, thirdPartyBadgesOn, hiddenBadgeProviders],
  );
  const snBadgeOn = thirdPartyBadgesOn && !badgeSourceHidden('streamnook');
  const atmosphere = atmosphereId ? getAtmosphere(atmosphereId) : null;
  // Frost behind the text only when the atmosphere declares it needs it (busy
  // washes); subtle ones render the text bare. A backdrop-filter per message row
  // is the single most expensive thing in the list on phone GPUs, so the wash
  // still renders there but the frost pass does not.
  const atmosphereFrost = features.richAtmospheres && !!atmosphere?.chatFrost;
  const cologneAtm = cologne ? getAtmosphere(MAJOR_COLOGNE_THEME_ID) : null;
  const [broadcasterType] = useState<string | null>(null);
  // Hover-action DOM (copy/pin cluster + mod menu) mounts on the row's FIRST
  // hover and stays mounted, so the CSS group-hover fade still runs on every
  // later hover. Before this, every row carried ~30-60 hidden nodes (mods:
  // the full punitive toolbar with tooltips) whether or not it was ever
  // hovered. Keyboard and drag moderation don't need this DOM (verified:
  // ChatModController invokes commands directly; drags target data-message-id).
  const [hoverArmed, setHoverArmed] = useState(false);
  // "Animate emotes: on hover" needs a live hover flag; the other modes never
  // write it, so rows do not re-render on pointer traffic unless asked.
  const animateEmotes = chatDesign?.animate_emotes ?? 'always';
  const [rowHovering, setRowHovering] = useState(false);
  // Twitch chat GIFs: shown, or a chip that reveals on click. Reveals are
  // per row and per GIF, and follow the Animate emotes mode the same way
  // animated emotes do (Never = chip, On hover = while hovered).
  const showChatGifs = chatDesign?.show_chat_gifs ?? true;
  const [revealedGifs, setRevealedGifs] = useState<Set<string>>(() => new Set());
  // Highlight, mention and reply-to-me are decided ONCE per message by the
  // Rust rule engine (src-tauri/src/services/chat_rules.rs) and stamped on
  // metadata; this row reads the stamp instead of running a regex loop per
  // row per window. Only optimistic local rows (our own sends, raw IRC
  // fallbacks) arrive unstamped; they cannot mention us, so the cheap
  // indexOf fallback below is for the raw-string path alone.
  const stamped = parsed.metadata?.rules_evaluated === true;
  const { isMentioned, isReplyToMe } = useMemo(() => {
    if (stamped) {
      return {
        isMentioned: parsed.metadata?.is_mentioned === true,
        isReplyToMe: parsed.metadata?.is_reply_to_me === true,
      };
    }
    if (!currentUser) return { isMentioned: false, isReplyToMe: false };
    const mentionTarget = `@${currentUser.username.toLowerCase()}`;
    const contentLower = parsed.content.toLowerCase();
    const mentionIndex = contentLower.indexOf(mentionTarget);
    let mentioned = false;
    if (mentionIndex !== -1) {
      const afterIndex = mentionIndex + mentionTarget.length;
      if (afterIndex >= contentLower.length) {
        mentioned = true;
      } else {
        const charAfter = contentLower[afterIndex];
        mentioned = /[\s.,!?:;'")\]}>]/.test(charAfter) || !/[a-z0-9_]/.test(charAfter);
      }
    }
    return {
      isMentioned: mentioned,
      isReplyToMe: parsed.replyInfo?.parentUserId === currentUser.user_id,
    };
  }, [stamped, parsed.metadata, parsed.content, parsed.replyInfo, currentUser]);

  // First matching phrase / user / badge rule, as stamped by Rust. Mentions
  // and replies suppress it there, so sound and animation never double-fire.
  const phraseMatch = useMemo<HighlightMatch | null>(() => {
    const hl = stamped ? parsed.metadata?.highlight : undefined;
    if (!hl) return null;
    return {
      phrase_id: hl.rule_id,
      color: hl.color,
      sound_id: (hl.sound_id ?? null) as HighlightMatch['sound_id'],
      cooldown_ms: hl.cooldown_ms,
    };
  }, [stamped, parsed.metadata]);

  // Fire the phrase's sound on first render if one is configured. Cooldown +
  // backfill guard (see notificationSound.ts) make this safe to call on every
  // matched message — historical replays don't trigger, and fast-repeating
  // matches are throttled per phrase.
  useEffect(() => {
    if (!phraseMatch?.sound_id) return;
    const sentTsRaw = parsed.tags.get('tmi-sent-ts');
    const sentTs = sentTsRaw ? parseInt(sentTsRaw, 10) : NaN;
    playSoundThrottled({
      key: phraseMatch.phrase_id,
      soundId: phraseMatch.sound_id,
      cooldownMs: phraseMatch.cooldown_ms,
      sentAtMs: Number.isFinite(sentTs) ? sentTs : null,
    });
    // phraseMatch is memoized on the message and the highlight settings, and
    // parsed.tags is fixed for the row, so this still fires once per match.
  }, [phraseMatch, parsed.tags]);

  // Window-title flash. Fires on any highlight match (phrase/user/badge) when
  // the user has opted in globally AND the window is currently blurred.
  // Backfill-safe: skip messages older than 5s so loading history doesn't
  // trigger a flash storm. Mention/reply path doesn't go through phraseMatch,
  // so this only covers the highlight cases (mentions get their own animation
  // and don't need title flash to attract attention).
  useEffect(() => {
    if (!phraseMatch) return;
    if (!chatHighlights?.appearance?.flash_title_when_unfocused) return;
    const sentTsRaw = parsed.tags.get('tmi-sent-ts');
    const sentTs = sentTsRaw ? parseInt(sentTsRaw, 10) : NaN;
    if (Number.isFinite(sentTs) && Date.now() - sentTs > 5000) return;
    const who = parsed.tags.get('display-name') || parsed.tags.get('login') || 'chat';
    flashTitle(`${who}: highlight`);
    // Toggling the setting can re-run this for a row under five seconds old;
    // the backfill guard above bounds that to one flash.
  }, [phraseMatch, parsed.tags, chatHighlights?.appearance?.flash_title_when_unfocused]);

  // All cosmetic resolution (paint, 7TV badge, third-party badges) lives in
  // chatUserStore.addUser. ChatMessage just subscribes via the selectors
  // above. No per-message useEffect / useState / fetch.

  // Reactive caching for 7TV cosmetics
  useEffect(() => {
    // Cache 7TV Badge - use id directly (BadgeV4 interface)
    if (seventvBadge?.id && !seventvBadge.localUrl) {
      const badgeUrl = getBadgeImageUrl(seventvBadge);
      if (badgeUrl && !badgeUrl.startsWith('asset') && !badgeUrl.includes('localhost')) {
        queueCosmeticForCaching(seventvBadge.id, badgeUrl);
      }
    }

    // Cache 7TV Paint Image Layers
    if (seventvPaint?.data?.layers) {
      seventvPaint.data.layers.forEach((layer: any) => {
        if (layer.ty.__typename === 'PaintLayerTypeImage' && layer.ty.images) {
          // Find the best image (scale 1 if available)
          const img = layer.ty.images.find((i: any) => i.scale === 1 && (layer.ty.images.some((x: any) => x.frameCount > 1) ? i.frameCount > 1 : true)) || layer.ty.images[0];

          if (img && !img.localUrl) {
            // Paints are cached by their layer ID
            queueCosmeticForCaching(layer.id, img.url);
          }
        }
      });
    }
  }, [seventvBadge, seventvPaint]);

  // Create username style with paint. effectiveColor is the override-aware
  // base color; the 7TV paint (if present) renders on top of it.
  const paintShadowMode = useAppStore((s) => s.settings.cosmetics?.paint_shadows) ?? 'all';

  const usernameStyle = useMemo(() => {
    if (!seventvPaint) {
      return { color: effectiveColor };
    }

    // Use the new computePaintStyle function
    return computePaintStyle(seventvPaint, effectiveColor, paintShadowMode);
  }, [seventvPaint, effectiveColor, paintShadowMode]);

  // Username prefix styling (normal messages only; action/"/me" stay plain).
  // The separator glyph sits between the name and the message; the name style
  // emphasizes the name itself. Color pulls from the chatter's own color or the
  // theme accent. username_colon is the deprecated boolean, migrated to 'colon'.
  const nameSeparator = chatDesign?.username_separator ?? (chatDesign?.username_colon ? 'colon' : 'none');
  const nameStyle = chatDesign?.username_style ?? 'plain';
  const prefixAccentColor = (chatDesign?.username_accent_source ?? 'user') === 'theme'
    ? 'var(--color-accent)'
    : (effectiveColor || 'var(--color-accent)');

  // How per-message mod actions are offered: classic click buttons, the
  // drag-to-moderate grab handle, or both. Migrates the old boolean.
  const modActionStyle = chatDesign?.mod_action_style
    ?? (chatDesign?.drag_moderation_enabled === false ? 'buttons' : 'both');
  // Drag-to-moderate is mod-only — every action bucket is a mod action, so a
  // non-mod dragging a message just lifts it into an empty void. Gate the whole
  // pickup gesture (and the text-selection sacrifice it requires) on the current
  // user's mod status, so non-mods always keep normal chat: select text, copy,
  // click links. The classic mod buttons are likewise already mod-gated below.
  const bodyDragEnabled = isModerator && (modActionStyle === 'drag' || modActionStyle === 'both');
  const showModButtons = modActionStyle === 'buttons' || modActionStyle === 'both';
  // The inline Pin button is ALWAYS available to moderators so a pin can never go
  // missing. The Pin Action setting only controls whether a Pin tile ALSO shows
  // in the drag-to-moderate gesture (handled in ModerationDragLayer).
  const showInlinePin = isModerator;
  // Is THIS message the one currently pinned? If so the inline control flips from
  // Pin to Unpin (and the same message can't show a redundant "pin" affordance).
  const thisMessageId = parsed.tags.get('id');
  const isThisPinned = usePinStore((s) => (!!thisMessageId && s.pinnedIds.includes(thisMessageId)));
  // While a mod drag is holding THIS message, sweep + tint it so it's clear
  // which message is about to be acted on.
  const draggedMessageId = useDragModerationStore((s) => s.dragged?.messageId);
  const isBeingDragged = !!draggedMessageId && draggedMessageId === parsed.tags.get('id');

  // Whole-message pickup: pressing on a non-interactive part of the row and
  // dragging past a small threshold lifts the chatter into the drag-to-moderate
  // buckets. Clicks and the things you'd normally click (name/badges/emotes/
  // links) are left alone; text selection is off in drag mode (use Copy).
  const handleBodyPickup = (e: React.PointerEvent) => {
    if (!bodyDragEnabled || e.button !== 0) return;
    const target = e.target as HTMLElement;
    if (target.closest('a, button, img, [data-no-drag]')) return;
    // Non-Twitch providers (Kick, YouTube) have no `user-id` tag; the chatter's id
    // rides on `providerUserId`. Twitch keeps the IRC tag.
    const dragUserId = parsed.provider === 'twitch' ? parsed.tags.get('user-id') : providerUserId;
    if (!dragUserId || !broadcasterId) return;
    const startX = e.clientX;
    const startY = e.clientY;
    const onMove = (me: PointerEvent) => {
      if (Math.hypot(me.clientX - startX, me.clientY - startY) < 6) return;
      teardown();
      useDragModerationStore.getState().startDrag(
        {
          userId: dragUserId,
          login: parsed.username,
          displayName: parsed.tags.get('display-name') || parsed.username,
          color: effectiveColor,
          messageId: parsed.tags.get('id') || undefined,
          broadcasterId,
          provider: parsed.provider,
          channel: parsed.channel?.replace(/^youtube:/, ''),
          isModerator,
          moderationState:
            moderationContext?.type === 'ban'
              ? 'ban'
              : moderationContext?.type === 'timeout'
                ? 'timeout'
                : null,
        },
        me.clientX,
        me.clientY,
      );
    };
    const onUp = () => teardown();
    function teardown() {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      window.removeEventListener('pointercancel', onUp);
      pickupTeardownRef.current = null;
    }
    // pointercancel covers gestures the browser takes over (touch scroll, pen),
    // which fire INSTEAD of pointerup; the ref lets an unmount tear down too.
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
    pickupTeardownRef.current = teardown;
  };

  const renderSegment = (
    segment: EmoteSegment,
    key: string,
    inGrid: boolean,
    isOverlay: boolean = false,
    // Modifier extras for a group's base member: growX stretches this
    // emote to double width (FFZ ffzW), wideX to a fixed 4:1 stretch (BTTV w!),
    // and rotated caps its width so a 90-degree turn cannot overflow into the
    // neighbouring chat lines. modifierNames lists the applied modifiers for
    // the tooltip (hidden modifiers render no img, so this is the only place
    // their presence is explained).
    mods?: { growX?: boolean; wideX?: boolean; rotated?: boolean; modifierNames?: string[] },
    // Gigantified render: 4x-size art and 8em sizing for the plucked emote
    // (and its whole group, so overlays keep covering their base).
    giant: boolean = false,
  ) => {
    const gridStyle = inGrid ? { gridArea: '1/1' } : {};
    const marginClass = inGrid ? '' : 'mx-0.5';
    
    if (segment.type === 'emote' && segment.isPersonal && chatDesign?.show_personal_emotes === false) {
      // A personal emote with the set turned off reads as the text it stands for.
      return <span key={key}>{segment.content}</span>;
    }
    if (segment.type === 'emote') {
      const emoteUrl = segment.emoteUrl ||
        (segment.emoteId ? `https://static-cdn.jtvnw.net/emoticons/v2/${segment.emoteId}/default/dark/2.0` : '');

      // Provider detection up front (reused for tiered caching, disk-first
      // render, srcSet, and the hover preview below). BTTV IDs are ALSO 24 hex
      // characters, so the other CDNs are excluded explicitly.
      const is7TVEmote = !!segment.emoteId && (
        emoteUrl.includes('7tv') ||
        ((segment.emoteId.length === 24 || segment.emoteId.length === 26) &&
          !emoteUrl.includes('jtvnw.net') &&
          !emoteUrl.includes('frankerfacez') &&
          !emoteUrl.includes('betterttv'))
      );
      const emoteTier = giant ? ('4x' as const) : inlineEmoteTier();

      // Resolve the real provider so the cache key matches the provider-
      // namespaced key the picker + prefetch write (otherwise disk-first
      // silently misses). 7TV stays per-tier; others key by `{provider}-{id}`.
      const emoteProvider = is7TVEmote
        ? '7tv'
        : emoteUrl.includes('jtvnw.net')
          ? 'twitch'
          : emoteUrl.includes('betterttv')
            ? 'bttv'
            : emoteUrl.includes('frankerfacez')
              ? 'ffz'
              : undefined;

      // Giant art URL. Only 7TV cache keys are per-tier; the other providers
      // key by bare id, so caching 4x bytes there would overwrite the
      // inline-size file (gigantified messages are rare, one CDN fetch is
      // fine).
      const displayUrl = giant
        ? giantEmoteCdnUrl(emoteUrl, emoteProvider, segment.emoteId)
        : emoteUrl;

      // Disk-first: prefer the on-disk copy at the rendered tier. On a miss,
      // queue it for caching (per-tier for 7TV) so the NEXT view is local.
      // This is what makes returning to a stream fast instead of re-pulling
      // every emote from the CDN.
      // Static mode: the CDN's first-frame file, bypassing the animated disk
      // cache (a static URL under the animated key would corrupt it).
      const wantStatic = !giant && (animateEmotes === 'never' || (animateEmotes === 'hover' && !rowHovering));
      const cachedEmoteUrl = !wantStatic && segment.emoteId && (!giant || emoteProvider === '7tv')
        ? getCachedEmoteUrl(segment.emoteId, emoteProvider, emoteTier)
        : undefined;
      if (!wantStatic && !cachedEmoteUrl && displayUrl && !displayUrl.startsWith('asset://') && !displayUrl.includes('asset.localhost') && segment.emoteId && (!giant || emoteProvider === '7tv')) {
        queueEmoteForDisplayCaching(segment.emoteId, emoteProvider, displayUrl, emoteTier);
      }

      // If it's an overlay (zero-width inside grid), it naturally sits on top and doesn't need negative margins
      // The grid "place-items-center" handles exact overlaying algorithmically!
      // srcSet lets the browser pick density from the CDN, but only when we are
      // NOT serving a fixed-size disk file (a local file is one resolution, so
      // srcSet would be meaningless and could re-pull from the network).
      let srcSet: string | undefined = undefined;
      if (!giant && !cachedEmoteUrl && is7TVEmote && segment.emoteId) {
        srcSet = `https://cdn.7tv.app/emote/${segment.emoteId}/1x.avif 1x, https://cdn.7tv.app/emote/${segment.emoteId}/2x.avif 2x, https://cdn.7tv.app/emote/${segment.emoteId}/3x.avif 3x, https://cdn.7tv.app/emote/${segment.emoteId}/4x.avif 4x`;
      }

      const displaySrc = wantStatic ? staticEmoteUrl(displayUrl) : cachedEmoteUrl || displayUrl;
      if (wantStatic) srcSet = is7TVEmote && segment.emoteId ? static7tvSrcSet(segment.emoteId) : undefined;

      const imgProps: React.ImgHTMLAttributes<HTMLImageElement> = {
        src: displaySrc,
        srcSet,
        alt: segment.content,
        loading: 'lazy',
        // Off-thread decode. Without this every emote in a busy chat decodes
        // synchronously at paint, on the same main thread hls.js appends from.
        decoding: 'async',
        className: `inline-block w-auto cursor-pointer ${inGrid ? '' : 'align-middle'} hover:scale-110 transition-transform ${isOverlay ? 'z-10 drop-shadow-[0_0_2px_rgba(0,0,0,0.5)] hover:drop-shadow-[0_0_4px_color-mix(in_srgb,var(--color-warning)_80%,transparent)]' : ''}`,
        style: {
          ...gridStyle,
          // In chat, size emotes in `em` so they scale with the message font;
          // the picker grid keeps its fixed rem size.
          height: giant
            ? GIANT_EMOTE_HEIGHT
            : inGrid
              ? 'calc(1.75rem * var(--sn-emote-scale, 1))'
              : 'calc(2em * var(--sn-emote-scale, 1))',
          maxWidth: giant
            ? GIANT_EMOTE_MAX_WIDTH
            : inGrid
              ? 'calc(128px * var(--sn-emote-scale, 1))'
              : 'calc(9em * var(--sn-emote-scale, 1))',
          ...(inGrid ? {} : { marginLeft: 'var(--sn-emote-margin, 0.125rem)', marginRight: 'var(--sn-emote-margin, 0.125rem)' }),
          // Must come last: a rotated emote is squared off first, exactly as
          // BetterTTV does it, so the turned image cannot spill vertically.
          ...(mods?.rotated ? { maxWidth: giant ? GIANT_EMOTE_HEIGHT : 'calc(1.75rem * var(--sn-emote-scale, 1))' } : {}),
        },
        referrerPolicy: 'no-referrer',
        onClick: () => {
          // Left-click a 7TV emote to open the quick spotlight modal, where it
          // can be added to any channel you edit (or escalated into the full
          // manager). Non-7TV emotes have no 7TV id, so they do nothing here.
          if (is7TVEmote && segment.emoteId) {
            useAppStore.getState().openEmoteSpotlight(segment.emoteId, segment.content);
          }
        },
        onContextMenu: (e) => {
          e.preventDefault();
          if (onEmoteRightClick) onEmoteRightClick(segment.content);
        },
        onError: (e) => {
          const t = e.currentTarget;
          // Giant 4x URL failed: retry once with the inline-size URL upscaled
          // (FFZ-equivalent degradation) before the cache/CDN ladder below.
          if (giant && displayUrl !== emoteUrl && !t.dataset.giantFallback) {
            t.dataset.giantFallback = '1';
            t.srcset = '';
            t.src = emoteUrl;
            return;
          }
          // A stale/missing disk file falls back to the CDN once (tier for
          // 7TV, base otherwise) before we give up and show the text code.
          if (cachedEmoteUrl && !t.dataset.cdnFallback) {
            t.dataset.cdnFallback = '1';
            if (is7TVEmote && segment.emoteId) {
              t.srcset = `https://cdn.7tv.app/emote/${segment.emoteId}/1x.avif 1x, https://cdn.7tv.app/emote/${segment.emoteId}/2x.avif 2x, https://cdn.7tv.app/emote/${segment.emoteId}/3x.avif 3x, https://cdn.7tv.app/emote/${segment.emoteId}/4x.avif 4x`;
              t.src = `https://cdn.7tv.app/emote/${segment.emoteId}/${emoteTier}.avif`;
            } else {
              t.src = emoteUrl;
            }
            return;
          }
          t.style.display = 'none';
          t.insertAdjacentText('afterend', segment.content);
        },
      };
      // BetterTTV's absolute stretch wins over FFZ's relative one when a
      // message stacks w! and ffzW on the same target.
      const imgElement = mods?.wideX ? (
        <WideEmoteImg
          {...imgProps}
          baseHeight={giant ? GIANT_EMOTE_HEIGHT : 'calc(1.75rem * var(--sn-emote-scale, 1))'}
        />
      ) : mods?.growX ? (
        <GrowXEmoteImg
          {...imgProps}
          baseHeight={giant ? GIANT_EMOTE_HEIGHT : 'calc(1.75rem * var(--sn-emote-scale, 1))'}
        />
      ) : (
        <img {...imgProps} />
      );

      // Big-preview tooltip mirroring the emote-picker's hover card. Compact
      // mode short-circuits to just the name (for users who find the
      // upscaled image distracting). The non-compact path now shows a 4x
      // preview, name, provider, optional Zero-Width chip, and the
      // "Right-click to copy" hint at the bottom so the copy affordance
      // still surfaces.
      const isCompactTooltip = !!chatDesign?.compact_emote_tooltips;
      // is7TVEmote was computed at the top of this branch (reused here).
      const providerLabel =
        is7TVEmote || emoteUrl.includes('7tv') ? '7TV'
        : emoteUrl.includes('betterttv') ? 'BetterTTV'
        : emoteUrl.includes('frankerfacez') ? 'FrankerFaceZ'
        : emoteUrl.includes('jtvnw.net') ? 'Twitch'
        : 'Emote';
      const previewUrl = is7TVEmote && segment.emoteId
        ? `https://cdn.7tv.app/emote/${segment.emoteId}/4x.avif`
        : emoteUrl;
      // User-configurable hover-preview height. Defaults to 96px (one step up
      // from the original fixed 64px preview). maxWidth scales with it so wide
      // 7TV emotes aren't clipped in the card.
      const hoverPreviewSize = chatDesign?.emote_hover_size ?? 96;
      const tooltipContent: React.ReactNode | string = isCompactTooltip
        ? segment.content
        : (
          <div className="flex flex-col items-center gap-1.5 py-0.5">
            <img
              src={previewUrl}
              alt={segment.content}
              className="w-auto object-contain mx-auto drop-shadow-md"
              style={{ height: hoverPreviewSize, maxWidth: hoverPreviewSize * 2 }}
              referrerPolicy="no-referrer"
              onError={(e) => {
                const t = e.currentTarget;
                // Some emotes have no working 4x (or a flaky avif), which is why
                // a Large preview sometimes blanked. Walk every size AND both
                // formats, then fall back to the cached inline-tier copy as a
                // last resort so the preview is never empty.
                if (is7TVEmote && segment.emoteId) {
                  const ladder = ['4x', '3x', '2x', '1x'].flatMap((s) => [
                    `https://cdn.7tv.app/emote/${segment.emoteId}/${s}.avif`,
                    `https://cdn.7tv.app/emote/${segment.emoteId}/${s}.webp`,
                  ]);
                  let step = Number(t.dataset.fb || '0');
                  while (step < ladder.length && ladder[step] === t.src) step++;
                  if (step < ladder.length) {
                    t.dataset.fb = String(step + 1);
                    t.src = ladder[step];
                    return;
                  }
                  if (cachedEmoteUrl && t.src !== cachedEmoteUrl) t.src = cachedEmoteUrl;
                }
              }}
            />
            <div className="text-center flex flex-col items-center gap-0.5">
              <span className="font-bold text-[13px] leading-tight">{segment.content}</span>
              <span className="text-[10px] text-white/60 leading-tight">{providerLabel}</span>
              {segment.modifierFlags != null ? (
                <span className="text-[9px] font-bold tracking-wider uppercase text-warning mt-0.5 mix-blend-screen drop-shadow-sm">
                  Modifier
                </span>
              ) : (
                segment.isZeroWidth && (
                  <span className="text-[9px] font-bold tracking-wider uppercase text-warning mt-0.5 mix-blend-screen drop-shadow-sm">
                    Zero-Width
                  </span>
                )
              )}
              {mods?.modifierNames && mods.modifierNames.length > 0 && (
                <span className="flex flex-wrap justify-center gap-1 mt-0.5">
                  {mods.modifierNames.map((n) => (
                    <span
                      key={n}
                      className="text-[9px] font-bold tracking-wider uppercase text-warning mix-blend-screen drop-shadow-sm"
                    >
                      {n}
                    </span>
                  ))}
                </span>
              )}
              <span className="text-[10px] text-white/50 mt-0.5">Right-click to copy</span>
            </div>
          </div>
        );
      return (
        <Tooltip key={key} content={tooltipContent} side="top">
          {isOverlay && !inGrid ? (
            // Fallback for standalone zero-width emote (e.g., at the start of a message)
            <span className="inline-block w-0 align-middle pointer-events-none" style={gridStyle}>
              <span className="pointer-events-auto -translate-x-full">
                {imgElement}
              </span>
            </span>
          ) : (
            imgElement
          )}
        </Tooltip>
      );
    }

    if (segment.type === 'emoji' && (segment.emojiUrl || segment.content)) {
      const emojiStyle = chatDesign?.emoji_style ?? 'apple';
      if (emojiStyle === 'system') {
        return <span key={key} className={marginClass}>{segment.content}</span>;
      }
      // Apple keeps the Rust-baked url and the on-disk cache; the other sets
      // are drawn straight from their CDN by codepoint.
      const emojiSrc =
        emojiStyle === 'apple' && segment.emojiUrl
          ? getCachedEmojiUrl(segment.content, segment.emojiUrl)
          : (vendorEmojiUrl(segment.content, emojiStyle) ?? segment.emojiUrl ?? '');
      return (
        <Tooltip key={key} content={segment.content} side="top">
          <img
            src={emojiSrc}
            alt={segment.content}
            className={`inline h-5 w-5 ${inGrid ? '' : 'align-middle'} ${marginClass}`}
            style={gridStyle}
            onError={(e) => {
              const t = e.currentTarget;
              // emoji-datasource-apple names some older text-default symbols
              // (clock, dove, heart, umbrella, etc.) WITH the -fe0f variation
              // selector in the filename, which our codepoint strips. Retry
              // once with it appended before giving up, so the real image loads
              // instead of leaving a blank.
              if (!t.dataset.fe0f && t.src.endsWith('.png') && !t.src.includes('-fe0f')) {
                t.dataset.fe0f = '1';
                t.src = t.src.replace(/\.png$/, '-fe0f.png');
                return;
              }
              // Truly unavailable: hide the broken image and drop in the native
              // emoji glyph so the slot is never blank.
              t.style.display = 'none';
              if (t.nextSibling?.textContent !== segment.content) {
                t.insertAdjacentText('afterend', segment.content);
              }
            }}
          />
        </Tooltip>
      );
    }

    if (segment.type === 'cheermote') {
      const bits = segment.bits ?? 0;
      return (
        <Tooltip key={key} content={`${bits.toLocaleString()} bits`} side="top">
          <span className={`inline-flex items-center gap-0.5 ${inGrid ? '' : 'align-middle'} ${marginClass}`} style={gridStyle}>
            <img
              src={segment.cheermoteUrl}
              alt={segment.content}
              className="inline-block h-7 w-auto align-middle"
              referrerPolicy="no-referrer"
              onError={(e) => {
                e.currentTarget.style.display = 'none';
                e.currentTarget.insertAdjacentText('afterend', segment.content);
              }}
            />
            <span className="font-bold text-sm" style={{ color: segment.color ?? '#979797' }}>
              {bits}
            </span>
          </span>
        </Tooltip>
      );
    }

    if (segment.type === 'gif' && segment.gifUrl) {
      // A Twitch chat GIF (GIPHY-backed, Tier 2/3 subscribers). Fixed height so
      // a late-loading image never shifts the list; width follows the asset.
      // Twitch requires the URL exactly as sent, and a GIF is one-off per
      // message, so it never enters the emote disk cache. The description
      // Twitch puts in the message text is the tooltip and the chip label.
      const gifKey = segment.gifId || segment.gifUrl;
      const label = segment.content.replace(/^\[|\]$/g, '');
      const reveal =
        showChatGifs &&
        (animateEmotes === 'always' ||
          (animateEmotes === 'hover' && rowHovering) ||
          revealedGifs.has(gifKey));
      if (!reveal) {
        return (
          <button
            key={key}
            type="button"
            title={label}
            onClick={() => setRevealedGifs((prev) => new Set(prev).add(gifKey))}
            className={`inline-flex max-w-[16rem] items-center gap-1 rounded-md border border-white/10 bg-white/5 px-1.5 py-0.5 text-xs text-textSecondary hover:bg-white/10 ${inGrid ? '' : 'align-middle'} ${marginClass}`}
            style={gridStyle}
          >
            <span className="font-semibold tracking-wide">GIF</span>
            <span className="truncate">{label}</span>
          </button>
        );
      }
      return (
        <Tooltip key={key} content={label} side="top">
          <img
            src={segment.gifUrl}
            alt={label}
            loading="lazy"
            decoding="async"
            referrerPolicy="no-referrer"
            className={`inline-block h-20 w-auto max-w-full rounded-md ${inGrid ? '' : 'align-middle'} ${marginClass}`}
            style={gridStyle}
            onError={(e) => {
              // Asset gone: keep the description Twitch sent instead of a blank.
              e.currentTarget.style.display = 'none';
              e.currentTarget.insertAdjacentText('afterend', segment.content);
            }}
          />
        </Tooltip>
      );
    }

    return (
      // Text stays on the baseline (no align-middle): middle-aligned text sits
      // a few px lower than the baseline-aligned username, which read as the
      // name floating above the message. Emote/emoji images keep their own
      // align-middle against this baseline.
      <span key={key} style={gridStyle}>
        {parseTextWithLinks(segment.content)}
      </span>
    );
  };

  // Phase 1: Group modifier and zero-width emotes with their target. FFZ
  // modifiers and 7TV/BTTV overlays attach BACKWARD (look-back, below);
  // BetterTTV modifiers attach FORWARD, so they are buffered until their
  // target arrives.
  const groupSegments = (segments: EmoteSegment[]): (EmoteSegment | EmoteSegment[])[] => {
    const groupedSegments: (EmoteSegment | EmoteSegment[])[] = [];

    const bttvModsOn = chatDesign?.bttv_emote_modifiers !== false;
    // Structural, NOT gated on the setting: a prefix modifier must never fall
    // through to the look-back below, or it would attach to the emote before
    // it, which is the bug this whole path exists to fix.
    const isPrefixMod = (s: EmoteSegment) =>
      s.type === 'emote' && ((s.modifierFlags ?? 0) & MOD_PREFIX) !== 0;
    const isSpacer = (s: EmoteSegment) => s.type === 'text' && s.content.trim() === '';
    // A modifier can never be another modifier's target.
    const isTarget = (s: EmoteSegment) =>
      (s.type === 'emote' || s.type === 'emoji') && s.modifierFlags == null;

    // Holds spacers as well as modifiers, so an unmatched run can be flushed
    // back verbatim and "w! hello" keeps its space.
    let pending: EmoteSegment[] = [];
    const flushPending = () => {
      // Straight into groupedSegments: an orphan prefix modifier renders as
      // its own plain art and must NOT enter the look-back branch.
      for (const p of pending) groupedSegments.push(p);
      pending = [];
    };

    segments.forEach((segment) => {
      if (isPrefixMod(segment)) {
        if (!bttvModsOn) {
          flushPending();
          groupedSegments.push(segment);
          return;
        }
        pending.push(segment);
        return;
      }

      if (pending.length > 0) {
        const last = pending[pending.length - 1];
        // The run survives exactly one spacer, matching BetterTTV: a doubled
        // space breaks attachment.
        if (isSpacer(segment) && !isSpacer(last)) {
          pending.push(segment);
          return;
        }
        if (isTarget(segment) && isSpacer(last)) {
          groupedSegments.push([segment, ...pending.filter(isPrefixMod)]);
          pending = [];
          return;
        }
        flushPending();
      }

      if (segment.type === 'emote' && segment.isZeroWidth) {
        if (groupedSegments.length > 0) {
          const last = groupedSegments[groupedSegments.length - 1];
          // If the last item is already a group, push into it
          if (Array.isArray(last)) {
            last.push(segment);
          } else if (last.type === 'text' && last.content.trim() === '') {
            // Twitch chat often puts spaces between typed emotes
            // If the last segment is just a space, check what's before the space
            if (groupedSegments.length > 1) {
              const beforeSpace = groupedSegments[groupedSegments.length - 2];
              if (Array.isArray(beforeSpace)) {
                beforeSpace.push(segment);
                groupedSegments.pop(); // Remove the space!
              } else if (beforeSpace.type === 'emote' || beforeSpace.type === 'emoji') {
                groupedSegments[groupedSegments.length - 2] = [beforeSpace, segment];
                groupedSegments.pop(); // Remove the space!
              } else {
                groupedSegments.push([segment]);
              }
            } else {
              groupedSegments.push([segment]);
            }
          } else if (last.type === 'emote' || last.type === 'emoji') {
            groupedSegments[groupedSegments.length - 1] = [last, segment];
          } else {
            groupedSegments.push([segment]);
          }
        } else {
          // Zero-width emote at the very start of the message
          groupedSegments.push([segment]);
        }
      } else {
        groupedSegments.push(segment);
      }
    });
    flushPending();
    return groupedSegments;
  };

  // Phase 2: Render groups. Member 0 is the target: it always draws its own
  // art and contributes no flags (an orphan modifier therefore renders as a
  // normal emote, matching both FFZ and BetterTTV). Members 1..n are the
  // attached overlays / modifiers: a modifier with the Hidden bit renders no
  // image and instead contributes its effect to the whole group.
  const ffzEffectsOn = chatDesign?.ffz_emote_effects !== false;
  // Which toggle owns a member is read off MOD_PREFIX: BetterTTV modifiers
  // always set it, FFZ modifiers never do. BetterTTV needs no check here
  // because Phase 1 already keeps its modifiers out of groups when its
  // toggle is off.
  const memberContributes = (seg: EmoteSegment) =>
    ((seg.modifierFlags ?? 0) & MOD_PREFIX) !== 0 || ffzEffectsOn;

  const renderGroup = (
    group: EmoteSegment | EmoteSegment[],
    index: number,
    giant: boolean = false,
  ): React.ReactNode => {
    if (Array.isArray(group)) {
      const aggFlags = group
        .slice(1)
        .reduce((acc, seg) => (memberContributes(seg) ? acc | (seg.modifierFlags ?? 0) : acc), 0);
      // Effect bits only. Hidden alone (legacy overlay modifiers) needs no
      // wrapper, and MOD_PREFIX is a routing marker rather than an effect,
      // so neither may force the wrapper path.
      const effectBits = aggFlags & ~MOD_NON_EFFECT_BITS;
      const growX = (effectBits & MOD_GROW_X) !== 0;
      const rotated = (effectBits & (MOD_BTTV_ROTATE_L | MOD_BTTV_ROTATE_R)) !== 0;
      // Rotating beats widening, because in BetterTTV the rotate rule's
      // max-width overrides the wide rule's width. Without this a rotated
      // wide emote would stand 4 lines tall and spill over its neighbours.
      const wideX = (effectBits & MOD_BTTV_WIDE) !== 0 && !rotated;
      const modifierNames = group
        .slice(1)
        .filter((s) => s.modifierFlags != null && memberContributes(s))
        .map((s) => s.content);

      const gridSpan = (
        <span
          key={effectBits ? undefined : `group-${index}`}
          className={`inline-grid items-center justify-items-center ${effectBits ? '' : 'align-middle mx-0.5'}`}
        >
          {group.map((seg, innerIndex) => {
            const hiddenModifier =
              innerIndex > 0 &&
              memberContributes(seg) &&
              ((seg.modifierFlags ?? 0) & MOD_HIDDEN) !== 0;
            if (hiddenModifier) return null;
            return renderSegment(
              seg,
              `${index}-${innerIndex}`,
              true,
              innerIndex > 0,
              innerIndex === 0
                ? {
                    growX,
                    wideX,
                    rotated,
                    modifierNames: modifierNames.length ? modifierNames : undefined,
                  }
                : undefined,
              giant,
            );
          })}
        </span>
      );
      if (!effectBits) return gridSpan;
      return wrapWithEmoteEffects(gridSpan, effectBits, `group-${index}`, index > 0);
    }
    return renderSegment(group, index.toString(), false, false, undefined, giant);
  };

  const renderContent = (segments: EmoteSegment[]) =>
    groupSegments(segments).map((g, i) => renderGroup(g, i));

  // Gigantified emote (Twitch "Gigantify an Emote" power-up, msg-id
  // gigantified-emote-message): the LAST non-emoji emote leaves the inline
  // flow and renders at 4x in a block below the body, matching Twitch and
  // FFZ. The pluck operates on GROUPS so zero-width overlays and FFZ/BTTV
  // modifiers attached to the emote move down with it instead of being
  // orphaned inline.
  const pluckGiantEmote = (
    segments: EmoteSegment[],
  ): { inline: React.ReactNode; giant: React.ReactNode | null } => {
    const groups = groupSegments(segments);
    // 'inline' keeps the emote in the text at its usual size.
    if (chatDesign?.giant_emote_align === 'inline') {
      return { inline: groups.map((g, i) => renderGroup(g, i)), giant: null };
    }
    let giantIdx = -1;
    for (let i = groups.length - 1; i >= 0; i--) {
      const g = groups[i];
      const base = Array.isArray(g) ? g[0] : g;
      // Emoji are never gigantified; a bare modifier is an attachment rather
      // than a target, and an orphan zero-width overlay must not become the
      // giant either.
      if (base.type === 'emote' && base.modifierFlags == null && !base.isZeroWidth) {
        giantIdx = i;
        break;
      }
    }
    if (giantIdx === -1) {
      return { inline: groups.map((g, i) => renderGroup(g, i)), giant: null };
    }
    return {
      inline: groups.filter((_, i) => i !== giantIdx).map((g, i) => renderGroup(g, i)),
      giant: renderGroup(groups[giantIdx], giantIdx, true),
    };
  };

  // Helper function to detect URLs and @mentions, making them styled and clickable
  const parseTextWithLinks = (text: string) => {
    // Combined regex: URLs and @mentions
    // @mentions: @username (alphanumeric + underscores, 1-25 chars per Twitch rules)
    const combinedRegex = /(https?:\/\/[^\s]+|www\.[^\s]+|@[a-zA-Z0-9_]{1,25})(?=\s|$|[.,!?:;'")\]}>-])/g;
    const parts = text.split(combinedRegex);

    return parts.map((part, index) => {
      // Check if this part is a URL
      if (part.match(/^(https?:\/\/|www\.)/)) {
        const url = part.startsWith('http') ? part : `https://${part}`;

        // In clean mode a preview card represents this link below the message —
        // drop the inline URL so it isn't duplicated above its own card. Only
        // trusted links auto-card, so only they are suppressed; untrusted links
        // keep their inline URL (their preview is click-to-load). In keep-link
        // mode we fall through and render the inline link too.
        if (!keepInlineLink && trustedPreviewUrls.includes(url)) return null;

        const shorten = chatDesign?.shorten_links ?? true;
        const label = shorten ? prettyUrlLabel(part) : part;
        const linkColor = (chatDesign?.link_color || '').trim();
        const linkUnderline = chatDesign?.link_underline !== false;

        const anchor = (
          <a
            key={index}
            href={url}
            // No target="_blank": the webview opens _blank links externally on
            // its own, which would stack a second tab on top of the open() below.
            className={`text-info hover:text-info/80 cursor-pointer${linkUnderline ? ' underline' : ''}`}
            style={linkColor ? { color: linkColor } : undefined}
            onClick={(e) => {
              // preventDefault runs BEFORE the open attempt, so if the open
              // fails there is no fallback navigation left - which is why this
              // has to route through a helper that actually works on both
              // platforms. The shell plugin's `open` rejects on Android (no
              // xdg-open), and the rejection used to be swallowed by a catch
              // that only logged: a dead tap on every chat link on the phone.
              e.preventDefault();
              void openExternal(url);
            }}
          >
            {label}
          </a>
        );

        // When shortened, surface the full URL on hover so the real destination
        // is always one hover away before clicking.
        return shorten && label !== url ? (
          <Tooltip key={index} content={url} side="top">
            {anchor}
          </Tooltip>
        ) : (
          anchor
        );
      }
      
      // Check if this part is an @mention
      if (part.match(/^@[a-zA-Z0-9_]{1,25}$/)) {
        const mentionedUsername = part.slice(1); // Remove @
        
        return (
          <MentionSpan
            key={index}
            username={mentionedUsername}
            onUsernameClick={onUsernameClick}
          />
        );
      }
      
      return part;
    });
  };

  // Check if this is a subscription message
  // Note: 'sharedchatnotice' is used for shared subscription events across channels
  const msgId = parsed.tags.get('msg-id');
  const sourceMsgId = parsed.tags.get('source-msg-id');
  const isSubscription = msgId === 'sub' ||
    msgId === 'resub' ||
    msgId === 'subgift' ||
    msgId === 'submysterygift' ||
    msgId === 'anonsubmysterygift' ||
    msgId === 'anonsubgift' ||
    msgId === 'sharedchatnotice' ||
    // YouTube channel memberships: a new/continuing member ('membership') and gifted
    // memberships ('membergift') reuse the same sub-card decoration via the system-msg
    // the adapter stamps. (Kick subs already speak 'sub'/'resub'.)
    msgId === 'membership' ||
    msgId === 'membergift' ||
    sourceMsgId === 'sub' ||
    sourceMsgId === 'resub' ||
    sourceMsgId === 'subgift' ||
    sourceMsgId === 'submysterygift' ||
    sourceMsgId === 'anonsubmysterygift' ||
    sourceMsgId === 'anonsubgift';

  // YouTube's paid highlights. They are donations with a comment attached, which
  // is exactly the shape of the charity-donation card, and the overlay already
  // classifies them alongside cheers/bits. Without this they fell through to a
  // PLAIN chat row, so a Super Chat read as ordinary text.
  const isSuperChat =
    msgId === 'superchat' ||
    msgId === 'supersticker' ||
    sourceMsgId === 'superchat' ||
    sourceMsgId === 'supersticker';
  const isSuperSticker = msgId === 'supersticker' || sourceMsgId === 'supersticker';

  // Check if this is a charity donation message
  const isDonation = msgId === 'charitydonation' || sourceMsgId === 'charitydonation' || isSuperChat;

  // Check if this is a viewer milestone (watch streak) message
  const isViewerMilestone = msgId === 'viewermilestone' || sourceMsgId === 'viewermilestone';
  const milestoneCategory = parsed.tags.get('msg-param-category');
  const isWatchStreak = isViewerMilestone && milestoneCategory === 'watch-streak';

  // Check if this is a highlighted message (channel points redemption)
  const isHighlightedMessage = msgId === 'highlighted-message' || sourceMsgId === 'highlighted-message';

  // Other chat-surfaced channel-points redemptions. They piggyback on the highlight
  // render path: same gradient background + a small label pill at the top right.
  // `custom-reward-id` is set on PRIVMSGs from custom rewards with "Skip the line for chat".
  // The named msg-id values cover the automatic message-style rewards.
  const customRewardId = parsed.tags.get('custom-reward-id');
  const hasCustomReward = !!customRewardId;
  // Cost + channel points icon for an injected no-input redemption row, shown as
  // "<reward name> <points glyph> <amount>" instead of a plain "(N)" in the text.
  const redemptionCost = parsed.tags.get('sn-reward-cost');
  const redemptionPointsIcon = parsed.tags.get('sn-points-icon');
  const isGigantifiedEmote =
    msgId === 'gigantified-emote-message' || sourceMsgId === 'gigantified-emote-message';
  const isAnimatedMessage =
    msgId === 'animated-message' || sourceMsgId === 'animated-message';
  const isSubModeBypass =
    msgId === 'skip-subs-mode-message' || sourceMsgId === 'skip-subs-mode-message';
  const redemptionLabel: string | null = isHighlightedMessage
    ? 'Redeemed Highlight My Message'
    : isGigantifiedEmote
      ? 'Sent a Gigantified Emote'
      : isAnimatedMessage
        ? 'Sent an Animated Message'
        : isSubModeBypass
          ? 'Sent a Message in Sub-Only Mode'
          : hasCustomReward
            ? 'Redeemed a channel points reward'
            : null;
  const isRedemption = redemptionLabel !== null;

  // Check if this is a bits cheer message
  const bitsAmount = parsed.tags.get('bits');
  // A cheer is its own card unless the viewer asked for it as a plain message,
  // in which case the row falls through with its cheermotes inline.
  const isBitsCheer = bitsAmount && parseInt(bitsAmount, 10) > 0 && (chatEvents?.cheer_display ?? 'card') !== 'message';

  // How event rows dress: the tinted cards, a ring, or nothing. The glint
  // classes are the same ones first-time rows use (see globals.css sn-ft-*).
  const eventStyle = chatEvents?.event_style ?? 'cards';
  const eventAnim =
    chatEvents?.event_animation && chatEvents.event_animation !== 'none' ? chatEvents.event_animation : null;
  const eventCardClass = (gradient: string) => {
    const base = eventStyle === 'plain' ? '' : eventStyle === 'outline' ? 'sn-event-outline' : gradient;
    const anim = eventAnim
      ? ` ${eventStyle === 'outline' ? 'sn-ft-anim-ring' : 'sn-ft-anim-bar'} sn-ft-t-${eventAnim}${chatEvents?.event_animate_repeat ? ' sn-ft-loop' : ''}`
      : '';
    return `relative ${base}${anim}`;
  };
  const eventCardStyle =
    eventStyle === 'outline' && chatEvents?.event_outline_color
      ? ({ '--sn-event-outline': chatEvents.event_outline_color } as React.CSSProperties)
      : undefined;
  // Custom wording for an event category, or null to keep the platform's.
  const eventTemplateFor = (category: 'subscription' | 'gift' | 'cheer' | 'milestone' | 'raid'): string | null => {
    const template = chatEvents?.event_templates?.[category];
    if (!template) return null;
    return renderEventTemplate(
      template,
      chatEventTemplateContext(parsed.tags, category, {
        username: parsed.username,
        displayName: parsed.tags.get('display-name') || parsed.displayName || parsed.username,
        provider: parsed.provider ?? 'twitch',
        bits: bitsAmount ? parseInt(bitsAmount, 10) : undefined,
        channel: (parsed.channel ?? '').split(':').pop() || undefined,
        time: parsed.metadata?.formatted_timestamp || undefined,
        defaultText: parsed.tags.get('system-msg')?.replace(/\\s/g, ' ') || undefined,
      }),
    );
  };

  // Check if this is a system message
  const isSystemMessage = parsed.username === 'System';

  // Get system message for subscriptions and donations
  const systemMessage = parsed.tags.get('system-msg')?.replace(/\\s/g, ' ');

  // Built-in event highlights (configurable per-event in Chat settings).
  // Defaults preserve prior behavior: first-time chatter ON (purple), all
  // others OFF. When ON, applies a tinted background + left border in the
  // configured color via the inline style stamp below.
  const builtInHighlights = chatHighlights?.built_in;
  // Tag first, metadata fallback: the overlay already accepts either
  // (OverlayChat), and backfilled or future paths may deliver one without
  // the other.
  const isFirstMessage =
    parsed.tags.get('first-msg') === '1' || parsed.metadata?.is_first_message === true;
  const isReturningChatter = parsed.tags.get('returning-chatter') === '1';
  const isOwnMessage = !!currentUser?.user_id && parsed.tags.get('user-id') === currentUser.user_id;
  const isRaidNotice = parsed.tags.get('msg-id') === 'raid';

  // Resolve which built-in event (if any) should drive the row's tint. Order
  // is intentional: raid > returning > first-time > self. Mention/reply flash
  // animations still win over all of these via the existing animationClass
  // cascade below.
  let builtInEventColor: string | null = null;
  let builtInEventLabel: string | null = null;
  const builtInStamp = stamped ? parsed.metadata?.built_in : undefined;
  let builtInKind: 'raider' | 'returning' | 'first_time' | 'self' | undefined;
  const suspiciousStatus = stamped ? parsed.metadata?.suspicious : undefined;
  const streamerModeActive = useStreamerMode((st) => st.active);
  if (stamped) {
    // Decided by the Rust rule engine with the same precedence and defaults.
    if (builtInStamp) {
      builtInEventColor = builtInStamp.color;
      builtInEventLabel = builtInStamp.label;
      builtInKind = builtInStamp.kind;
    }
  } else if (isRaidNotice && (builtInHighlights?.raider?.enabled ?? false)) {
    builtInEventColor = builtInHighlights?.raider?.color ?? '#ef4444';
    builtInEventLabel = 'Raid';
  } else if (isReturningChatter && (builtInHighlights?.returning_chatter?.enabled ?? false)) {
    builtInEventColor = builtInHighlights?.returning_chatter?.color ?? '#22d3ee';
    builtInEventLabel = 'Returning chatter';
  } else if (isFirstMessage && (builtInHighlights?.first_time_chatter?.enabled ?? true)) {
    // Default ON for backwards compatibility — first-time chatter previously
    // had a hardcoded purple gradient. When the user re-colors it via the
    // settings panel, the new color flows through builtInEventColor.
    builtInEventColor = builtInHighlights?.first_time_chatter?.color ?? '#a855f7';
    builtInEventLabel = 'First message in chat';
    builtInKind = 'first_time';
  } else if (isOwnMessage && (builtInHighlights?.self_message?.enabled ?? false)) {
    builtInEventColor = builtInHighlights?.self_message?.color ?? '#facc15';
    builtInEventLabel = 'You';
  }
  // /announce rows: Twitch sends msg-id=announcement with a colour name. Tint
  // the row in that colour and label it, like the web client's banner.
  if (!builtInEventColor && (msgId === 'announcement' || sourceMsgId === 'announcement')) {
    const paramColor = (parsed.tags.get('msg-param-color') || 'PRIMARY').toUpperCase();
    builtInEventColor =
      paramColor === 'BLUE' ? '#3b82f6'
      : paramColor === 'GREEN' ? '#22c55e'
      : paramColor === 'ORANGE' ? '#f97316'
      : paramColor === 'PURPLE' ? '#a855f7'
      : (chatDesign?.mention_color ?? '#9147ff');
    builtInEventLabel = 'Announcement';
  }
  // Low-trust chatter (EventSub suspicious_user, moderators only): the tint
  // marks it even when no other built-in applies.
  if (!builtInEventColor && suspiciousStatus) {
    builtInEventColor = suspiciousStatus === 'restricted' ? '#ef4444' : '#f59e0b';
    builtInEventLabel = suspiciousStatus === 'restricted' ? 'Restricted user' : 'Monitored user';
  }

  // Extract source room info for shared chat (needed for all message types)
  const sourceRoomId = parsed.tags.get('source-room-id');
  const currentRoomId = parsed.tags.get('room-id');
  const isFromSharedChat = sourceRoomId && currentRoomId && sourceRoomId !== currentRoomId;

  // State to store the fetched channel name - initialize from cache if available
  // These hooks must be declared before any conditional returns
  const [fetchedChannelName, setFetchedChannelName] = useState<string | null>(() => {
    if (sourceRoomId && channelNameCache.has(sourceRoomId)) {
      return channelNameCache.get(sourceRoomId) || null;
    }
    return null;
  });

  // State to store the channel profile image
  const [channelProfileImage, setChannelProfileImage] = useState<string | null>(() => {
    if (sourceRoomId && channelProfileImageCache.has(sourceRoomId)) {
      return channelProfileImageCache.get(sourceRoomId) || null;
    }
    return null;
  });

  // Fetch source channel name and profile image if this is a shared chat message (only once per sourceRoomId)
  useEffect(() => {
    if (!isFromSharedChat || !sourceRoomId) return;

    // Both caches were read by the state initializers (sourceRoomId is fixed
    // for a row), so a cache hit means there is nothing to fetch.
    if (channelProfileImageCache.has(sourceRoomId)) return;

    let isMounted = true;

    // Fetch the channel name and profile image
    loadInvoke().then((invoke) => {
      invoke<any>('get_user_by_id', { userId: sourceRoomId })
        .then((user) => {
          if (!isMounted) return;
          if (user && user.login) {
            // Store name in cache
            channelNameCache.set(sourceRoomId, user.login);
            setFetchedChannelName(user.login);

            // Store profile image in cache
            if (user.profile_image_url) {
              channelProfileImageCache.set(sourceRoomId, user.profile_image_url);
              setChannelProfileImage(user.profile_image_url);
            }
          }
        })
        .catch((err) => {
          Logger.warn('[ChatMessage] Failed to fetch source channel info:', err);
        });
    });

    return () => {
      isMounted = false;
    };
  }, [isFromSharedChat, sourceRoomId, fetchedChannelName, channelProfileImage]);

  // Handle bits cheers
  if (isBitsCheer) {
    const cheerTemplate = eventTemplateFor('cheer');
    // Generate a unique key based on message ID to prevent animation restarts
    const messageId = parsed.tags.get('id') || `bits-${parsed.username}-${parsed.tags.get('tmi-sent-ts') ?? ''}`;
    const bitsCount = parseInt(bitsAmount!, 10);

    // Format bits count with commas for readability
    const formattedBits = bitsCount.toLocaleString();

    // Defensive: EventSub shows gigantified messages arrive with cheer:null,
    // but `bits` is a plain IRC tag; if both ever co-occur, the giant must
    // not be silently dropped by this early return.
    const giantPluckCheer =
      isGigantifiedEmote && chatDesign?.giant_emotes !== false
        ? pluckGiantEmote(contentWithEmotes)
        : null;

    // Determine bits tier color based on amount (matching Twitch's color scheme)
    const getBitsTierColor = (bits: number): string => {
      if (bits >= 10000) return '#ff1f1f'; // Red
      if (bits >= 5000) return '#0099fe'; // Blue
      if (bits >= 1000) return '#1db2a6'; // Teal
      if (bits >= 100) return '#9c3ee8'; // Purple
      return '#979797'; // Gray (1-99)
    };

    const bitsTierColor = getBitsTierColor(bitsCount);

    // Twitch's own animated bits gem for this tier (same CDN pattern the Rust
    // cheermote parser builds); tier thresholds mirror getBitsTierColor.
    const bitsTier =
      bitsCount >= 10000 ? '10000' : bitsCount >= 5000 ? '5000' : bitsCount >= 1000 ? '1000' : bitsCount >= 100 ? '100' : '1';
    const bitsAnimUrl = `https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/${bitsTier}/2.gif`;

    // Helper function to render username as clickable. Render path uses the
    // effective display name (so nicknames apply); the click payload keeps
    // the real display name so the profile card opens to the true identity.
    const renderClickableUsername = (username: string, displayName?: string) => {
      const userIdForClick = clickUserId;
      return (
        <Tooltip content="Click to view profile" side="top">
          <span
            className="font-bold cursor-pointer hover:underline"
            style={usernameStyle}
            onClick={(e) => {
              if (userIdForClick && onUsernameClick) {
                onUsernameClick(
                  userIdForClick,
                  username,
                  displayName || username,
                  parsed.color,
                  parsed.badges,
                  e,
                  clickProvider
                );
              }
            }}
          >
            {effectiveDisplayName || displayName || username}
          </span>
        </Tooltip>
      );
    };

    // Helper function to render badges
    const renderBadges = () => {
      const senderUserId = parsed.tags.get('user-id');
      const isSN = isStreamNookUser(senderUserId);

      if (visibleBadges.length === 0 && !seventvBadge && thirdPartyBadges.length === 0 && !isSN) return null;

      return (
        <span className="inline-flex items-center gap-1 mr-1">
          {visibleBadges.map((badge, idx) => {
            // Handle both old format (key/info) and new format (name/version)
            if (!badge.info) return null;
            return (
              <Tooltip key={`bits-badge-${badge.key}-${idx}`} content={badge.info.title} side="top">
                <img
                  src={getTwitchBadgeUrl(badge.key, badge.info)}
                  alt={badge.info.title}
                  className="sn-chat-badge inline-block cursor-pointer hover:scale-110 transition-transform"
                  onClick={() => onBadgeClick?.(badge.key, badge.info)}
                  onError={(e) => {
                    Logger.warn('[Badge] Failed to load badge:', badge.key, badge.info.image_url_1x);
                    e.currentTarget.style.display = 'none';
                  }}
                />
              </Tooltip>
            );
          })}
          {seventvBadge && (
            <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
              <button
                onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
                className="inline-block cursor-pointer hover:scale-110 transition-transform"
              >
                <FallbackImage
                  src={getBadgeImageUrl(seventvBadge)}
                  fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                  alt={seventvBadge.description || seventvBadge.name}
                  className="sn-chat-badge inline-block"
                />
              </button>
            </Tooltip>
          )}
          {thirdPartyBadges.filter(badge => badge && badge.imageUrl).map((badge, idx) => (
            <Tooltip key={`bits-tp-badge-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
              <img
                // 2x, not imageUrl's 4x; see the default layout's badge row.
                src={badge.image2x || badge.imageUrl}
                alt={badge.title}
                className="sn-chat-badge inline-block"
                onError={(e) => {
                  e.currentTarget.style.display = 'none';
                }}
              />
            </Tooltip>
          ))}
          {isSN && snBadgeOn && <StreamNookBadge userId={senderUserId} userNumber={getStreamNookUserNumber(senderUserId)} />}
        </span>
      );
    };

    // Use dynamic spacing from user settings
    const eventPadding = calculateHalfPadding(chatDesign?.message_spacing ?? 8);

    return (
      <div key={messageId} className={`px-3 border-t border-borderSubtle ${eventCardClass('bits-gradient')}`} style={{ ...eventCardStyle, paddingTop: `${eventPadding}px`, paddingBottom: `${eventPadding}px` }}>
        <div className="flex items-center gap-2.5">
          <div className="flex-shrink-0">
            {/* Twitch's animated bits gem for the tier */}
            <img
              src={bitsAnimUrl}
              alt=""
              className="w-5 h-5 object-contain"
              decoding="async"
              referrerPolicy="no-referrer"
              onError={(e) => { e.currentTarget.style.display = 'none'; }}
            />
          </div>
          <div
            className="flex-1 min-w-0 flex flex-col leading-relaxed"
            style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}
          >
            <p className="text-white font-semibold leading-relaxed">
              {renderBadges()}
              {cheerTemplate ? (
                <span style={{ color: bitsTierColor }} className="font-bold">{cheerTemplate}</span>
              ) : (
                <>
                  {renderClickableUsername(parsed.username, parsed.tags.get('display-name') || parsed.username)}
                  <span style={{ color: bitsTierColor }} className="font-bold"> cheered {formattedBits} bits</span>
                </>
              )}
            </p>
            {parsed.content && (
              <p className="text-textSecondary mt-1 leading-relaxed break-words">
                {giantPluckCheer ? giantPluckCheer.inline : renderContent(contentWithEmotes)}
              </p>
            )}
            {giantPluckCheer?.giant && (
              <div className={`mt-1 flex ${giantJustify}`}>{giantPluckCheer.giant}</div>
            )}
          </div>
        </div>
      </div>
    );
  }

  // Handle charity donations
  if (isDonation) {
    // Generate a unique key based on message ID to prevent animation restarts
    const messageId = parsed.tags.get('id') || `donation-${parsed.username}-${parsed.tags.get('tmi-sent-ts') ?? ''}`;

    // Get donation details
    const charityName = parsed.tags.get('msg-param-charity-name')?.replace(/\\s/g, ' ');
    const donationAmount = parsed.tags.get('msg-param-donation-amount');
    const donationCurrency = parsed.tags.get('msg-param-donation-currency') || 'USD';
    const exponent = parseInt(parsed.tags.get('msg-param-exponent') || '2', 10);

    // Calculate the actual donation amount (amount is in smallest currency unit)
    const actualAmount = donationAmount ? (parseInt(donationAmount, 10) / Math.pow(10, exponent)) : 0;

    // Format the amount with currency symbol
    const twitchAmount = new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: donationCurrency,
    }).format(actualAmount);
    // YouTube carries its own amount tags (`sc-currency` is already a symbol, so it
    // is prepended rather than run through Intl, which would need an ISO code).
    // Falling back to the stamped system message keeps the card correct even if a
    // future payload omits the numeric tags.
    const scCurrency = parsed.tags.get('sc-currency') ?? '';
    const scAmount = parsed.tags.get('sc-amount');
    // Prefer YouTube's own formatted string: rebuilding it from the numeric tags
    // drops the currency's conventions ("$2.00" becomes "$2").
    const superChatAmount =
      parsed.tags.get('sc-display') || (scAmount ? `${scCurrency}${scAmount}` : null);
    let formattedAmount = isSuperChat ? (superChatAmount ?? '') : twitchAmount;
    // Shown in the viewer's currency when they asked for one and the amount can
    // be read: the original stays when the rate is not loaded yet or the
    // currency is unknown.
    const targetCurrency = (chatEvents?.superchat_currency || '').trim().toUpperCase();
    if (targetCurrency) {
      preloadRates();
      const fromCode = isSuperChat
        ? symbolToCode(scCurrency) ?? (scCurrency.length === 3 ? scCurrency.toUpperCase() : undefined)
        : donationCurrency.toUpperCase();
      const rawAmount = isSuperChat ? Number.parseFloat((scAmount ?? '').replace(/[^0-9.]/g, '')) : actualAmount;
      if (fromCode && Number.isFinite(rawAmount) && rawAmount > 0 && fromCode !== targetCurrency) {
        const converted = convertMoney(rawAmount, fromCode, targetCurrency);
        if (converted !== null) formattedAmount = formatMoney(converted, targetCurrency);
      }
    }
    // The adapter prefixes the message body with "Super Chat - $2.00  " so a plain
    // row still reads correctly. This card puts the amount in its header, so render
    // the RAW comment instead or the amount appears twice.
    const superChatComment = parsed.tags.get('sc-message') ?? '';

    // Check if this is a shared chat notice (from another channel)
    const isSharedChat = msgId === 'sharedchatnotice';
    const isFromDifferentChannel = isSharedChat && isFromSharedChat;

    // Helper function to render username as clickable. Render path uses the
    // effective display name (so nicknames apply); the click payload keeps
    // the real display name so the profile card opens to the true identity.
    const renderClickableUsername = (username: string, displayName?: string) => {
      const userIdForClick = clickUserId;
      return (
        <Tooltip content="Click to view profile" side="top">
          <span
            className="font-bold cursor-pointer hover:underline"
            style={usernameStyle}
            onClick={(e) => {
              if (userIdForClick && onUsernameClick) {
                onUsernameClick(
                  userIdForClick,
                  username,
                  displayName || username,
                  parsed.color,
                  parsed.badges,
                  e,
                  clickProvider
                );
              }
            }}
          >
            {effectiveDisplayName || displayName || username}
          </span>
        </Tooltip>
      );
    };

    // Helper function to render badges
    const renderBadges = () => {
      const senderUserId = parsed.tags.get('user-id');
      const isSN = isStreamNookUser(senderUserId);

      if (visibleBadges.length === 0 && !seventvBadge && thirdPartyBadges.length === 0 && !isSN) return null;

      return (
        <span className="inline-flex items-center gap-1 mr-1">
          {visibleBadges.map((badge, idx) => {
            if (!badge.info) return null;
            return (
              <Tooltip key={`donation-badge-${badge.key}-${idx}`} content={badge.info.title} side="top">
                <img
                  src={getTwitchBadgeUrl(badge.key, badge.info)}
                  alt={badge.info.title}
                  className="sn-chat-badge inline-block cursor-pointer hover:scale-110 transition-transform"
                  onClick={() => onBadgeClick?.(badge.key, badge.info)}
                  onError={(e) => {
                    Logger.warn('[Badge] Failed to load badge:', badge.key, badge.info.image_url_1x);
                    e.currentTarget.style.display = 'none';
                  }}
                />
              </Tooltip>
            );
          })}
          {seventvBadge && (
            <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
              <button
                onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
                className="inline-block cursor-pointer hover:scale-110 transition-transform"
              >
                <FallbackImage
                  src={getBadgeImageUrl(seventvBadge)}
                  fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                  alt={seventvBadge.description || seventvBadge.name}
                  className="sn-chat-badge inline-block"
                />
              </button>
            </Tooltip>
          )}
          {thirdPartyBadges.filter(badge => badge && badge.imageUrl).map((badge, idx) => (
            <Tooltip key={`donation-tp-badge-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
              <img
                // 2x, not imageUrl's 4x; see the default layout's badge row.
                src={badge.image2x || badge.imageUrl}
                alt={badge.title}
                className="sn-chat-badge inline-block"
                onError={(e) => {
                  e.currentTarget.style.display = 'none';
                }}
              />
            </Tooltip>
          ))}
          {isSN && snBadgeOn && <StreamNookBadge userId={senderUserId} userNumber={getStreamNookUserNumber(senderUserId)} />}
        </span>
      );
    };

    // Use dynamic spacing from user settings
    const eventPadding = calculateHalfPadding(chatDesign?.message_spacing ?? 8);

    return (
      <div key={messageId} className={`px-3 border-t border-borderSubtle ${eventCardClass('donation-gradient')}`} style={{ ...eventCardStyle, paddingTop: `${eventPadding}px`, paddingBottom: `${eventPadding}px` }}>
        {/* Shared chat indicator */}
        {isFromDifferentChannel && (
          <div className="flex items-center gap-1.5 mb-2 pb-2 border-b border-borderSubtle">
            <svg className="w-3.5 h-3.5 text-accent flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
            </svg>
            <span className="text-xs text-accent font-semibold">From Shared Chat</span>
            {fetchedChannelName && (
              <>
                <span className="text-xs text-textSecondary">-</span>
                <Tooltip content={`Switch to ${fetchedChannelName}'s stream`} side="top">
                  <button
                    onClick={async () => {
                      try {
                        await useAppStore.getState().startStream(fetchedChannelName);
                      } catch (err) {
                        Logger.error('[ChatMessage] Failed to switch to shared channel:', err);
                        useAppStore.getState().addToast(`Failed to switch to ${fetchedChannelName}'s stream`, 'error');
                      }
                    }}
                    className="text-xs text-info font-semibold hover:underline cursor-pointer"
                  >
                    {fetchedChannelName}
                  </button>
                </Tooltip>
              </>
            )}
          </div>
        )}

        <div className="flex items-center gap-2.5">
          <div className="flex-shrink-0">
            {isSuperChat ? (
              /* Coin: a heart reads as charity, which a Super Chat is not. */
              <svg className="w-5 h-5 text-success" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.94 6.94a.75.75 0 011.06 0l2.5 2.5a.75.75 0 010 1.06l-2.5 2.5a.75.75 0 11-1.06-1.06L10.38 10 8.94 8.56a.75.75 0 010-1.06z" clipRule="evenodd" />
              </svg>
            ) : (
              /* Heart/Charity icon */
              <svg className="w-5 h-5 text-success" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" d="M3.172 5.172a4 4 0 015.656 0L10 6.343l1.172-1.171a4 4 0 115.656 5.656L10 17.657l-6.828-6.829a4 4 0 010-5.656z" clipRule="evenodd" />
              </svg>
            )}
          </div>
          <div
            className="flex-1 min-w-0 flex flex-col leading-relaxed"
            style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}
          >
            <p className="text-white font-semibold leading-relaxed">
              {renderBadges()}
              {renderClickableUsername(parsed.username, parsed.tags.get('display-name') || parsed.username)}
              {isSuperChat ? (
                <span className="text-success font-bold">
                  {formattedAmount
                    ? ` sent a ${formattedAmount} ${isSuperSticker ? 'Super Sticker' : 'Super Chat'}`
                    : ` sent a ${isSuperSticker ? 'Super Sticker' : 'Super Chat'}`}
                </span>
              ) : (
                <>
                  <span className="text-success font-bold"> donated {formattedAmount}</span>
                  {charityName && <span className="text-textSecondary"> to support {charityName}</span>}
                </>
              )}
            </p>
            {isSuperChat
              ? superChatComment && (
                  <p className="text-textSecondary mt-1 leading-relaxed break-words">
                    {superChatComment}
                  </p>
                )
              : parsed.content && (
                  <p className="text-textSecondary mt-1 leading-relaxed break-words">
                    {renderContent(contentWithEmotes)}
                  </p>
                )}
          </div>
        </div>
      </div>
    );
  }

  // Handle watch streak milestone messages
  if (isWatchStreak) {
    // Generate a unique key based on message ID to prevent animation restarts
    const messageId = parsed.tags.get('id') || `watchstreak-${parsed.username}-${parsed.tags.get('tmi-sent-ts') ?? ''}`;

    // Get watch streak details
    const streakValue = parsed.tags.get('msg-param-value'); // Number of consecutive streams
    const channelPointsReward = parsed.tags.get('msg-param-copoReward'); // Channel points earned
    const displayName = parsed.tags.get('display-name') || parsed.username;

    // Helper function to render username as clickable
    const renderClickableUsername = (username: string, displayNameProp?: string) => {
      const userIdForClick = clickUserId;
      return (
        <Tooltip content="Click to view profile" side="top">
          <span
            className="font-bold cursor-pointer hover:underline"
            style={usernameStyle}
            onClick={(e) => {
              if (userIdForClick && onUsernameClick) {
                onUsernameClick(
                  userIdForClick,
                  username,
                  displayNameProp || username,
                  parsed.color,
                  parsed.badges,
                  e,
                  clickProvider
                );
              }
            }}
          >
            {effectiveDisplayName || displayNameProp || username}
          </span>
        </Tooltip>
      );
    };

    // Helper function to render badges
    const renderBadges = () => {
      const senderUserId = parsed.tags.get('user-id');
      const isSN = isStreamNookUser(senderUserId);

      if (visibleBadges.length === 0 && !seventvBadge && thirdPartyBadges.length === 0 && !isSN) return null;

      return (
        <span className="inline-flex items-center gap-1 mr-1">
          {visibleBadges.map((badge, idx) => {
            if (!badge.info) return null;
            return (
              <Tooltip key={`watchstreak-badge-${badge.key}-${idx}`} content={badge.info.title} side="top">
                <img
                  src={getTwitchBadgeUrl(badge.key, badge.info)}
                  alt={badge.info.title}
                  className="sn-chat-badge inline-block cursor-pointer hover:scale-110 transition-transform"
                  onClick={() => onBadgeClick?.(badge.key, badge.info)}
                  onError={(e) => {
                    Logger.warn('[Badge] Failed to load badge:', badge.key, badge.info.image_url_1x);
                    e.currentTarget.style.display = 'none';
                  }}
                />
              </Tooltip>
            );
          })}
          {seventvBadge && (
            <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
              <button
                onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
                className="inline-block cursor-pointer hover:scale-110 transition-transform"
              >
                <FallbackImage
                  src={getBadgeImageUrl(seventvBadge)}
                  fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                  alt={seventvBadge.description || seventvBadge.name}
                  className="sn-chat-badge inline-block"
                />
              </button>
            </Tooltip>
          )}
          {thirdPartyBadges.filter(badge => badge && badge.imageUrl).map((badge, idx) => (
            <Tooltip key={`watchstreak-tp-badge-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
              <img
                // 2x, not imageUrl's 4x; see the default layout's badge row.
                src={badge.image2x || badge.imageUrl}
                alt={badge.title}
                className="sn-chat-badge inline-block"
                onError={(e) => {
                  e.currentTarget.style.display = 'none';
                }}
              />
            </Tooltip>
          ))}
          {isSN && snBadgeOn && <StreamNookBadge userId={senderUserId} userNumber={getStreamNookUserNumber(senderUserId)} />}
        </span>
      );
    };

    // Use dynamic spacing from user settings
    const eventPadding = calculateHalfPadding(chatDesign?.message_spacing ?? 8);

    return (
      <div key={messageId} className={`px-3 border-t border-borderSubtle ${eventCardClass('watchstreak-gradient')}`} style={{ ...eventCardStyle, paddingTop: `${eventPadding}px`, paddingBottom: `${eventPadding}px` }}>
        <div className="flex items-center gap-2.5">
          <div className="flex-shrink-0">
            {/* Fire/Watch Streak icon from Twitch */}
            <svg className="w-5 h-5 text-highlight-orange" viewBox="0 0 20 20" fill="currentColor">
              <path fillRule="evenodd" d="M11 4.5 9 2 4.8 6.9A7.48 7.48 0 0 0 3 11.77C3 15.2 5.8 18 9.23 18h1.65A6.12 6.12 0 0 0 17 11.88c0-1.86-.65-3.66-1.84-5.1L12 3l-1 1.5ZM6.32 8.2 9 5l2 2.5L12 6l1.62 2.07A5.96 5.96 0 0 1 15 11.88c0 2.08-1.55 3.8-3.56 4.08.36-.47.56-1.05.56-1.66 0-.52-.18-1.02-.5-1.43L10 11l-1.5 1.87c-.32.4-.5.91-.5 1.43 0 .6.2 1.18.54 1.64A4.23 4.23 0 0 1 5 11.77c0-1.31.47-2.58 1.32-3.57Z" clipRule="evenodd" />
            </svg>
          </div>
          <div
            className="flex-1 min-w-0 flex flex-col"
            style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}
          >
            <p className="text-white font-semibold leading-relaxed flex items-center flex-wrap gap-1">
              {renderBadges()}
              {renderClickableUsername(parsed.username, displayName)}
              {channelPointsReward && (
                <>
                  {/* Channel Points icon */}
                  <svg className="w-4 h-4 text-highlight-orange inline-block ml-1" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M12 5v2a5 5 0 0 1 5 5h2a7 7 0 0 0-7-7Z"></path>
                    <path fillRule="evenodd" d="M1 12C1 5.925 5.925 1 12 1s11 4.925 11 11-4.925 11-11 11S1 18.075 1 12Zm11 9a9 9 0 1 1 0-18 9 9 0 0 1 0 18Z" clipRule="evenodd"></path>
                  </svg>
                  <span className="text-highlight-orange font-bold">+{parseInt(channelPointsReward, 10).toLocaleString()}</span>
                </>
              )}
            </p>
            <p className="text-textSecondary mt-0.5 leading-relaxed break-words">
              {eventTemplateFor('milestone') ?? `Watched ${streakValue} consecutive streams and sparked a watch streak!`}
            </p>
            {parsed.content && (
              <p className="text-textPrimary mt-1 leading-relaxed break-words">
                {renderContent(contentWithEmotes)}
              </p>
            )}
          </div>
        </div>
      </div>
    );
  }

  if (isSubscription) {
    // Generate a unique key based on message ID to prevent animation restarts
    const messageId = parsed.tags.get('id') || `sub-${parsed.username}-${parsed.tags.get('tmi-sent-ts') ?? ''}`;

    // Get subscription details
    const subMsgId = parsed.tags.get('msg-id');
    const msgParamRecipientDisplayName = parsed.tags.get('msg-param-recipient-display-name');
    const msgParamSubPlan = parsed.tags.get('msg-param-sub-plan');
    const msgParamMonths = parsed.tags.get('msg-param-cumulative-months') || parsed.tags.get('msg-param-months');
    const msgParamMassGiftCount = parsed.tags.get('msg-param-mass-gift-count');

    // Check if this is a shared chat notice (from another channel)
    const isSharedChat = subMsgId === 'sharedchatnotice';
    const isFromDifferentChannel = isSharedChat && isFromSharedChat;

    // Helper function to render badges
    const renderBadges = () => {
      const senderUserId = parsed.tags.get('user-id');
      const isSN = isStreamNookUser(senderUserId);

      if (visibleBadges.length === 0 && !seventvBadge && thirdPartyBadges.length === 0 && !isSN) return null;

      return (
        <span className="inline-flex items-center align-middle gap-1 mr-1">
          {visibleBadges.map((badge, idx) => {
            if (!badge.info) return null;
            return (
              <Tooltip key={`sub-badge-${badge.key}-${idx}`} content={badge.info.title} side="top">
                <img
                  src={getTwitchBadgeUrl(badge.key, badge.info)}
                  alt={badge.info.title}
                  className="sn-chat-badge inline-block cursor-pointer hover:scale-110 transition-transform"
                  onClick={() => onBadgeClick?.(badge.key, badge.info)}
                  onError={(e) => {
                    Logger.warn('[Badge] Failed to load badge:', badge.key, badge.info.image_url_1x);
                    e.currentTarget.style.display = 'none';
                  }}
                />
              </Tooltip>
            );
          })}
          {seventvBadge && (
            <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
              <button
                onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
                className="inline-block cursor-pointer hover:scale-110 transition-transform"
              >
                <FallbackImage
                  src={getBadgeImageUrl(seventvBadge)}
                  fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                  alt={seventvBadge.description || seventvBadge.name}
                  className="sn-chat-badge inline-block"
                />
              </button>
            </Tooltip>
          )}
          {thirdPartyBadges.filter(badge => badge && badge.imageUrl).map((badge, idx) => (
            <Tooltip key={`sub-tp-badge-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
              <img
                // 2x, not imageUrl's 4x; see the default layout's badge row.
                src={badge.image2x || badge.imageUrl}
                alt={badge.title}
                className="sn-chat-badge inline-block"
                onError={(e) => {
                  e.currentTarget.style.display = 'none';
                }}
              />
            </Tooltip>
          ))}
          {isSN && snBadgeOn && <StreamNookBadge userId={senderUserId} userNumber={getStreamNookUserNumber(senderUserId)} />}
        </span>
      );
    };

    // Helper function to parse system message and make usernames clickable
    const parseSystemMessageWithClickableNames = (message: string) => {
      if (!message) return null;

      // Pattern to match usernames in the system message
      // This will match the subscriber's name and recipient names
      const usernamePattern = /\b([A-Za-z0-9_]{3,25})\b/g;
      const parts: (string | JSX.Element)[] = [];
      let lastIndex = 0;
      let match;
      let keyIndex = 0;

      // Get recipient info from tags
      const recipientUserId = parsed.tags.get('msg-param-recipient-id');
      const recipientUserName = parsed.tags.get('msg-param-recipient-user-name');
      const recipientDisplayName = msgParamRecipientDisplayName;

      while ((match = usernamePattern.exec(message)) !== null) {
        const matchedName = match[1];

        // Add text before the match
        if (match.index > lastIndex) {
          parts.push(message.substring(lastIndex, match.index));
        }

        // Check if this is the subscriber's username or recipient's username
        const isSubscriber = matchedName.toLowerCase() === parsed.username.toLowerCase();
        const isRecipient = recipientUserName &&
          matchedName.toLowerCase() === recipientUserName.toLowerCase();

        if (isSubscriber) {
          // Gifter with their badges and cosmetics
          parts.push(
            <span
              key={`username-${keyIndex++}`}
              className="inline-flex items-center align-middle"
              style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}
            >
              {renderBadges()}
              <Tooltip content="Click to view profile" side="top">
                <span
                  className="font-bold cursor-pointer hover:underline"
                  style={usernameStyle}
                  onClick={(e) => {
                    if (userId && onUsernameClick) {
                      onUsernameClick(
                        userId,
                        parsed.username,
                        parsed.tags.get('display-name') || parsed.username,
                        parsed.color,
                        parsed.badges,
                        e
                      );
                    }
                  }}
                >
                  {effectiveDisplayName || matchedName}
                </span>
              </Tooltip>
            </span>
          );
        } else if (isRecipient && recipientUserId) {
          // Recipient with their own cosmetics
          parts.push(
            <UsernameWithCosmetics
              key={`username-${keyIndex++}`}
              username={recipientUserName}
              userIdProp={recipientUserId}
              displayName={recipientDisplayName}
              userOverrides={userOverrides}
              paintShadowMode={paintShadowMode}
              onUsernameClick={onUsernameClick}
            />
          );
        } else {
          // Keep as plain text
          parts.push(matchedName);
        }

        lastIndex = match.index + matchedName.length;
      }

      // Add remaining text
      if (lastIndex < message.length) {
        parts.push(message.substring(lastIndex));
      }

      return parts.length > 0 ? parts : message;
    };

    // Build a more detailed message for different subscription types
    let displayMessage = systemMessage;

    if (!displayMessage) {
      // Fallback messages if system-msg is not available
      if (msgId === 'sub') {
        displayMessage = `${parsed.username} subscribed!`;
      } else if (msgId === 'resub') {
        displayMessage = `${parsed.username} subscribed for ${msgParamMonths} months!`;
      } else if (msgId === 'subgift') {
        displayMessage = `${parsed.username} gifted a subscription to ${msgParamRecipientDisplayName}!`;
      } else if (msgId === 'submysterygift') {
        displayMessage = `${parsed.username} is gifting ${msgParamMassGiftCount} subscriptions to the community!`;
      } else if (msgId === 'anonsubmysterygift') {
        displayMessage = `An anonymous user is gifting ${msgParamMassGiftCount} subscriptions to the community!`;
      } else if (msgId === 'anonsubgift') {
        displayMessage = `An anonymous user gifted a subscription to ${msgParamRecipientDisplayName}!`;
      } else {
        displayMessage = `${parsed.username} subscribed!`;
      }
    }
    // The viewer's own wording wins when every token it names is present.
    const subTemplate = eventTemplateFor(categoryOf(msgId) === 'gift' ? 'gift' : 'subscription');
    if (subTemplate) displayMessage = subTemplate;

    // Use dynamic spacing from user settings
    const eventPadding = calculateHalfPadding(chatDesign?.message_spacing ?? 8);

    return (
      <div key={messageId} className={`px-3 border-t border-borderSubtle ${eventCardClass('subscription-gradient')}`} style={{ ...eventCardStyle, paddingTop: `${eventPadding}px`, paddingBottom: `${eventPadding}px` }}>
        {/* Shared chat indicator */}
        {isFromDifferentChannel && (
          <div className="flex items-center gap-1.5 mb-2 pb-2 border-b border-borderSubtle">
            <svg className="w-3.5 h-3.5 text-accent flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z" />
            </svg>
            <span className="text-xs text-accent font-semibold">From Shared Chat</span>
            {fetchedChannelName && (
              <>
                <span className="text-xs text-textSecondary">-</span>
                <Tooltip content={`Switch to ${fetchedChannelName}'s stream`}>
                <button
                  onClick={async () => {
                    try {

                      // Use the startStream method to switch to the shared channel
                      await useAppStore.getState().startStream(fetchedChannelName);
                    } catch (err) {
                      Logger.error('[ChatMessage] Failed to switch to shared channel:', err);
                      useAppStore.getState().addToast(`Failed to switch to ${fetchedChannelName}'s stream`, 'error');
                    }
                  }}
                  className="text-xs text-info font-semibold hover:underline cursor-pointer"
                >
                  {fetchedChannelName}
                </button>
                </Tooltip>
              </>
            )}
          </div>
        )}

        <div className="flex items-center gap-2.5">
          <div className="flex-shrink-0">
            {/* Prime logo for Prime subs, Gift box for other subs */}
            {msgParamSubPlan === 'Prime' ? (
              <svg className="w-5 h-5 text-info" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" clipRule="evenodd" d="M18 5v8a2 2 0 0 1-2 2H4a2.002 2.002 0 0 1-2-2V5l4 3 4-4 4 4 4-3z" />
              </svg>
            ) : (
              <Gift size={20} className="text-accent" />
            )}
          </div>
          <div
            className="flex-1 min-w-0 flex flex-col leading-relaxed"
            style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}
          >
            <p className="text-white font-semibold leading-relaxed">
              {parseSystemMessageWithClickableNames(displayMessage)}
            </p>
            {(subMsgId === 'submysterygift' || subMsgId === 'anonsubmysterygift') && giftRecipients.length > 0 && (
              <div className="text-textSecondary text-xs mt-1 leading-relaxed break-words">
                <span className="mr-1">
                  {giftExpected
                    ? `${giftRecipients.length} of ${giftExpected} gifted to`
                    : `Gifted to`}
                </span>
                {(giftRecipientsExpanded ? giftRecipients : giftRecipients.slice(0, 8)).map((r, i, arr) => (
                  <span key={r.userId}>
                    <UsernameWithCosmetics username={r.userName} userIdProp={r.userId} displayName={r.displayName} userOverrides={userOverrides} paintShadowMode={paintShadowMode} onUsernameClick={onUsernameClick} />
                    {i < arr.length - 1 ? <span>, </span> : null}
                  </span>
                ))}
                {!giftRecipientsExpanded && giftRecipients.length > 8 && (
                  <button
                    type="button"
                    onClick={() => setGiftRecipientsExpanded(true)}
                    className="ml-1 text-accent hover:underline cursor-pointer"
                  >
                    +{giftRecipients.length - 8} more
                  </button>
                )}
              </div>
            )}
            {parsed.content && (
              <p className="text-textSecondary mt-1 leading-relaxed break-words">
                {renderContent(contentWithEmotes)}
              </p>
            )}
          </div>
        </div>
      </div>
    );
  }

  // Handle system messages directly to apply strict yellow styling without username
  if (isSystemMessage) {
    const eventPadding = calculateHalfPadding(chatDesign?.message_spacing ?? 8);
    const messageId = parsed.tags.get('id') || `system-${parsed.tags.get('tmi-sent-ts') ?? parsed.content.slice(0, 40)}`;
    
    return (
      <div 
        key={messageId} 
        className="px-3 border-y border-warning/20 bg-warning/10 mb-[1px]"
        style={{ paddingTop: `${eventPadding}px`, paddingBottom: `${eventPadding}px` }}
      >
        <div className="flex items-center gap-2">
          {/* Information / Alert Icon */}
          <div className="flex-shrink-0">
            <svg className="w-4 h-4 text-warning/80" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
            </svg>
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-warning font-medium text-xs leading-relaxed">
              {renderContent(contentWithEmotes)}
            </p>
          </div>
        </div>
        {typeof message !== 'string' && message.songCard && (
          <div className="mt-1.5 ml-6">
            <SongCard card={message.songCard} />
          </div>
        )}
      </div>
    );
  }


  // Build dynamic styles based on chat design settings
  // Use consistent padding on container - spacing between messages is handled via py classes
  const messageSpacing = chatDesign?.message_spacing ?? 8;
  const messageStyle: React.CSSProperties = {
    paddingTop: `${Math.max(4, messageSpacing / 2)}px`,
    paddingBottom: `${Math.max(4, messageSpacing / 2)}px`,
  };

  // Giant emote pluck for the main render path (the cheer branch above
  // computes its own). OFF, or no pluckable emote, leaves the render
  // identical to the pre-feature output.
  const giantPluck =
    isGigantifiedEmote && chatDesign?.giant_emotes !== false
      ? pluckGiantEmote(contentWithEmotes)
      : null;

  // Determine animation class and border color. Mentions/replies to you take
  // precedence over phrase/built-in highlights. The flash is gated on the
  // mention_animation toggle, but the chosen color always paints the left
  // border so it still shows when the flash is off.
  let animationClass = '';
  let borderLeftColor = '';
  const flashOn = chatDesign?.mention_animation !== false;

  if (isMentioned) {
    borderLeftColor = chatDesign?.mention_color ?? '#ff4444';
    if (flashOn) animationClass = 'animate-mention-flash';
  } else if (isReplyToMe) {
    borderLeftColor = chatDesign?.reply_color ?? '#ff6b6b';
    if (flashOn) animationClass = 'animate-reply-flash';
  } else if (phraseMatch) {
    animationClass = 'animate-phrase-highlight';
    borderLeftColor = phraseMatch.color;
  } else if (isHighlighted) {
    animationClass = 'animate-highlight-flash';
  }

  // Alternating row backgrounds are handled in CSS (.chat-striped > row:nth-child)
  // so they cost zero React re-renders. See ChatMessageList + globals.css.

  // Build border class based on settings
  // Divider anchored to the TOP of each message instead of the bottom. That
  // way the very last visible message has no line at its bottom edge, so
  // when a new message arrives, the "absolute bottom" of the chat has no
  // 1px line that needs to move with the layout. Removes a tiny vertical
  // shimmer at the bottom edge.
  const borderClass = chatDesign?.show_dividers !== false ? 'border-t border-borderSubtle' : '';

  // StreamNook badge: regular-message render path (the 5th badge render site).
  // Bits, donation, watchstreak, and subscription messages each have their own
  // renderBadges() helper above; everything else (regular text, replies, actions,
  // mentions, and the user's own outgoing messages) flows through the JSX below.
  const senderUserId = parsed.tags.get('user-id');
  const isSN = isStreamNookUser(senderUserId);

  // Global highlight appearance — applies to BOTH built-in event highlights
  // and the phrase/user/badge match (phraseMatch). Defaults preserve prior
  // visual: standard display style with ~20% opacity background.
  const appearance = chatHighlights?.appearance;
  const displayStyle = appearance?.display_style ?? 'standard';
  const tintOpacityPct = Math.max(0, Math.min(100, appearance?.opacity ?? 20));
  // Hex alpha 00-ff. 20% → 0x33, 100% → 0xff, 0% → 0x00.
  const tintAlphaHex = Math.round((tintOpacityPct / 100) * 255)
    .toString(16)
    .padStart(2, '0');
  const showHighlightBg = displayStyle === 'standard';
  const showHighlightBorder = displayStyle !== 'none';

  // Compose built-in event tint styles inline. The configurable color drives
  // a gradient (alpha controlled by tintOpacityPct). Phrase/mention/reply
  // flash still take precedence on the left border (their borderLeftColor
  // overrides below).
  // First-time rows can wear a ring instead of the wash, and a one-shot glint.
  const ftRule = builtInHighlights?.first_time_chatter;
  const ftRing = builtInKind === 'first_time' && ftRule?.style === 'ring' && !!builtInEventColor;
  const ftAnim = builtInKind === 'first_time' && ftRule?.animation && ftRule.animation !== 'none' ? ftRule.animation : null;
  const ftClass = `${ftRing ? ` sn-ft-ring${ftRule?.fill ? ' sn-ft-fill' : ''}` : ''}${
    ftAnim ? ` ${ftRing ? 'sn-ft-anim-ring' : 'sn-ft-anim-bar'} sn-ft-t-${ftAnim}${ftRule?.animate_repeat ? ' sn-ft-loop' : ''}` : ''
  }`;
  const builtInEventBg =
    builtInEventColor && showHighlightBg && !ftRing
      ? `linear-gradient(to right, ${builtInEventColor}${tintAlphaHex}, ${builtInEventColor}${Math.round(
          (tintOpacityPct / 100) * 128,
        )
          .toString(16)
          .padStart(2, '0')}, transparent)`
      : undefined;

  return (
    <div
      className={`group relative isolate px-3 hover:bg-glass transition-colors ${borderClass} ${animationClass
        }${ftClass} ${isRedemption ? 'highlight-message-gradient' : ''
        } ${isFromSharedChat ? 'border-l-2 border-l-accent/50 bg-accent/5' : ''
        } ${moderationContext && (chatDesign?.deleted_message_style ?? 'strikethrough') !== 'keep' ? 'opacity-50' : ''} ${bodyDragEnabled ? 'select-none cursor-grab' : ''} ${isBeingDragged ? 'overflow-hidden' : ''}`}
      style={{
        ...messageStyle,
        ...(builtInEventBg ? { backgroundImage: builtInEventBg } : {}),
        ...(ftRing ? ({ '--sn-ft-color': builtInEventColor } as React.CSSProperties) : {}),
        borderLeftColor: (isMentioned || isReplyToMe)
          ? borderLeftColor
          : phraseMatch && showHighlightBorder
            ? phraseMatch.color
            : builtInEventColor && showHighlightBorder && !ftRing
              ? builtInEventColor
              : undefined,
        borderLeftWidth:
          isMentioned || isReplyToMe || ((phraseMatch || (builtInEventColor && !ftRing)) && showHighlightBorder)
            ? '4px'
            : undefined,
        borderLeftStyle:
          ((builtInEventColor && !ftRing) || phraseMatch) && showHighlightBorder && !(isMentioned || isReplyToMe)
            ? 'solid'
            : undefined,
        ...(phraseMatch
          ? ({ '--phrase-flash-color': phraseMatch.color } as React.CSSProperties)
          : {}),
        ...((isMentioned || isReplyToMe) && borderLeftColor
          ? ({ '--flash-accent-color': borderLeftColor } as React.CSSProperties)
          : {}),
        // The Cologne frame needs horizontal room + a minimum height so the gold
        // border is never cut off on a single-line message; the background-only
        // and coin variants need no special padding.
        ...(cologne?.frame
          ? { paddingLeft: 18, paddingRight: 18, paddingTop: 7, paddingBottom: 7, minHeight: 36 }
          : {}),
      }}
      onPointerDown={handleBodyPickup}
      onPointerEnter={(e) => {
        if (!hoverArmed) setHoverArmed(true);
        if (animateEmotes === 'hover' && e.pointerType !== 'touch') setRowHovering(true);
      }}
      onPointerLeave={animateEmotes === 'hover' ? () => setRowHovering(false) : undefined}
    >
      {/* Atmosphere wash: the same animated aurora as the member's profile
          backdrop, masked to fade out before the text so it stays readable. */}
      {cologne && cologneAtm ? (
        <MajorCologneChrome
          textureUrl={cologneAtm.chromeTexture ?? ''}
          coinUrl={cologneAtm.chromeCoin}
          frameUrl={cologneAtm.chromeFrame}
          coin={cologne.coin}
          frame={cologne.frame}
        />
      ) : atmosphere ? (
        <AtmosphereBackground atm={atmosphere} variant="chat" />
      ) : null}

      {/* Held-in-drag marker: a faint accent wash + a repeating light sweep so
          you're sure which message is in your cursor. */}
      {isBeingDragged && (
        <>
          <div aria-hidden="true" className="pointer-events-none absolute inset-0 z-20 bg-accent/10" />
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-0 z-20"
            style={{
              background: 'linear-gradient(105deg, transparent 42%, rgba(255,255,255,0.18) 50%, transparent 58%)',
              animation: 'sn-held-sweep 2s ease-in-out infinite',
            }}
          />
        </>
      )}

      {/* "Grab to drag" hint: an accent bar down the left edge that fades in on
          hover whenever whole-message drag is enabled (pairs with the grab
          cursor). Full-height accent + soft shadow so it pops over the Atmosphere
          and on any theme. */}
      {bodyDragEnabled && (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-y-1 left-0 z-20 w-1.5 rounded-r-md bg-accent shadow-[1px_0_4px_rgba(0,0,0,0.45)] opacity-0 group-hover:opacity-100 transition-opacity duration-150"
        />
      )}

      {/* Built-in event label (first-time chatter, returning, self, raid) */}
      {builtInEventLabel && (
        <div className="flex items-center justify-end gap-1.5">
          <span
            className="text-xs font-normal opacity-60"
            style={{ color: builtInEventColor ?? undefined }}
          >
            {builtInEventLabel}
          </span>
        </div>
      )}
      {/* Channel points redemption indicator (highlight, gigantify, animate, skip-subs-mode, custom reward) */}
      {redemptionLabel && (
        <div className="flex items-center justify-end gap-1">
          <svg className="w-3 h-3 text-highlight-cyan opacity-60" viewBox="0 0 20 20" fill="currentColor">
            <path d="M10 6a4 4 0 014 4h-2a2 2 0 00-2-2V6z" />
            <path fillRule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-2 0a6 6 0 11-12 0 6 6 0 0112 0z" clipRule="evenodd" />
          </svg>
          <span className="text-xs text-highlight-cyan font-normal opacity-60">{redemptionLabel}</span>
        </div>
      )}
      {/* Reply indicator */}
      {parsed.replyInfo && replyStyle === 'full' && (
        <Tooltip content="Click to view parent message" side="top">
          <div
            className="mb-1.5 pl-2 border-l-2 border-textSecondary/40 cursor-pointer hover:border-textSecondary/60 transition-colors"
            // This line owns its own tap. Without the marker, a tap on mobile
            // ALSO reaches MobileChatPane's tap-to-reply handler on the list, so
            // jumping to the parent would simultaneously start a reply to the
            // child. `data-no-drag` is the existing repo convention for exactly
            // this (see the mod-drag handles below and StyledChatName), and it
            // is inert on desktop, which has no competing parent click handler.
            data-no-drag="true"
            onClick={() => onReplyClick?.(parsed.replyInfo!.parentMsgId)}
          >
            <div
              className="flex items-center gap-1.5 text-textSecondary"
              // Scale the reply preview with the chat font (~85% of it, matching the
              // old text-xs at the default 14px) instead of a fixed size.
              style={{ fontSize: `${Math.round((chatDesign?.font_size ?? 14) * 0.85)}px` }}
            >
              <svg className="flex-shrink-0" style={{ width: '1.1em', height: '1.1em' }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 10h10a8 8 0 018 8v2M3 10l6 6m-6-6l6-6" />
              </svg>
              <span className="font-semibold">{getDisplayedName(parsed.replyInfo.parentUserId, parsed.replyInfo.parentDisplayName, userOverrides)}</span>
              <span className="truncate flex-1">
                {replyPreviewSegments
                  ? replyPreviewSegments.map((seg, i) =>
                      (seg.type === 'emote' || seg.type === 'emoji') && (seg.emoteUrl || seg.emojiUrl) ? (
                        <img
                          key={`rp-${i}`}
                          src={seg.emoteUrl || seg.emojiUrl}
                          alt={seg.content}
                          className="inline-block align-middle mx-px"
                          style={{ height: '1.35em' }}
                        />
                      ) : (
                        seg.content
                      ),
                    )
                  : parsed.replyInfo.parentMsgBody}
              </span>
            </div>
          </div>
        </Tooltip>
      )}

      <div>
        {/* Timestamp - own line above the badges/name/message when enabled */}
        {formattedTimestamp && (
          <div
            className={`text-[10px] leading-tight mb-0.5 text-textSecondary ${
              atmosphereFrost
                ? 'block w-fit rounded-sm bg-[rgba(5,6,13,0.2)] px-1 opacity-70 backdrop-blur-[4px]'
                : 'opacity-50'
            }`}
          >
            {formattedTimestamp}
          </div>
        )}
        {/* Badges and Message content - inline flow so wrapped text starts at left edge.
            When the atmosphere asks for frost (chatFrost) this is one frosted
            block hugging the badges + name + message (the timestamp gets its
            own separate frost above), so the text/badges stay readable over a
            busy wash. */}
        <div className={atmosphereFrost ? 'inline-block max-w-full rounded-md bg-[rgba(5,6,13,0.22)] px-1.5 py-0.5 backdrop-blur-[4px]' : 'min-w-0'}>
          {/* YouTube / TikTok native inline avatar — leads the row (before badges)
              so it shows even for chatters with no badges. The picture rides every
              message (those adapters stamp it onto the `avatar` tag). */}
          {chatDesign?.show_avatars !== false && (parsed.provider === 'youtube' || parsed.provider === 'tiktok') && parsed.tags.get('avatar') && (
            <img
              src={parsed.tags.get('avatar')}
              alt=""
              className="inline-block rounded-full mr-1.5 align-middle object-cover"
              style={{
                // Scale with the chat font size (≈20px at the 14px default).
                width: Math.round((chatDesign?.font_size ?? 14) * 1.4),
                height: Math.round((chatDesign?.font_size ?? 14) * 1.4),
              }}
              onError={(e) => {
                e.currentTarget.style.display = 'none';
              }}
            />
          )}
          {/* Badges */}
          {isSN || (isFromSharedChat && channelProfileImage) || visibleBadges.length > 0 || seventvBadge || thirdPartyBadges.length > 0 ? (
            <span className="inline-flex items-center gap-1 mr-1.5 align-middle">
              {/* Shared chat channel profile image badge */}
              {isFromSharedChat && channelProfileImage && (
                <Tooltip content={`Chatting from ${fetchedChannelName || 'shared channel'}`} side="top">
                  <img
                    src={channelProfileImage}
                    alt={`${fetchedChannelName || 'Channel'} profile`}
                    className="w-5 h-5 cursor-pointer hover:scale-110 transition-transform object-cover"
                    onClick={async () => {
                      if (fetchedChannelName) {
                        try {
                          await useAppStore.getState().startStream(fetchedChannelName);
                        } catch (err) {
                          Logger.error('[ChatMessage] Failed to switch to shared channel:', err);
                        }
                      }
                    }}
                    onError={(e) => {
                      Logger.warn('[Badge] Failed to load channel profile image');
                      e.currentTarget.style.display = 'none';
                    }}
                  />
                </Tooltip>
              )}
              {/* Twitch badges, channel-contextual (subscriber, poll, …) before global. */}
              {orderTwitchBadges(visibleBadges).map((badge, idx) => {
                if (!badge.info) return null;
                return (
                  <Tooltip key={`${badge.key}-${idx}`} content={badge.info.title} side="top">
                    <img
                      src={getTwitchBadgeUrl(badge.key, badge.info)}
                      alt={badge.info.title}
                      className="sn-chat-badge cursor-pointer hover:scale-110 transition-transform"
                      onClick={() => onBadgeClick?.(badge.key, badge.info)}
                      onError={(e) => {
                        // Hide broken badge images
                        Logger.warn('[Badge] Failed to load badge:', badge.key, badge.info.image_url_1x);
                        e.currentTarget.style.display = 'none';
                      }}
                    />
                  </Tooltip>
                );
              })}
              {seventvBadge && (
                <Tooltip content={`Click for details: ${seventvBadge.description || seventvBadge.name}`} side="top">
                  <button
                    onClick={() => openBadgesWithBadgeInMain(seventvBadge.id)}
                    className="inline-block cursor-pointer hover:scale-110 transition-transform"
                  >
                    <FallbackImage
                      src={getBadgeImageUrl(seventvBadge)}
                      fallbackUrls={getBadgeFallbackUrls(seventvBadge.id).slice(1)}
                      alt={seventvBadge.description || seventvBadge.name}
                      className="sn-chat-badge"
                    />
                  </button>
                </Tooltip>
              )}
              {/* Third-party badges (FFZ, Chatterino, Homies, Moltorino, ...). The
                  row is a 20px box, so the 2x variant is the right one: `imageUrl`
                  is the 4x file, which for Moltorino is a 72px, 120-frame, ~960KB
                  animated webp per badge. Member-loadout badges carry one URL for
                  every size, so the fallback keeps them unchanged. */}
              {thirdPartyBadges.filter(badge => badge && badge.imageUrl).map((badge, idx) => (
                <Tooltip key={`tp-badge-${badge.id}-${idx}`} content={`${badge.title} (${badge.provider.toUpperCase()})`} side="top">
                  <img
                    src={badge.image2x || badge.imageUrl}
                    alt={badge.title}
                    className="sn-chat-badge"
                    onError={(e) => {
                      e.currentTarget.style.display = 'none';
                    }}
                  />
                </Tooltip>
              ))}
              {/* StreamNook identity badge sits rightmost, next to the name (see utils/badgeOrder). */}
              {isSN && snBadgeOn && <StreamNookBadge userId={senderUserId} userNumber={getStreamNookUserNumber(senderUserId)} />}
            </span>
          ) : null}

          {/* Message content */}
          <span
            className="leading-relaxed align-middle"
            style={{
              fontSize: `${chatDesign?.font_size ?? 14}px`,
              fontWeight: chatDesign?.font_weight ?? 400,
            }}
          >
            {isAction ? (
              // ACTION messages: entire content in username color
              <span style={{ ...usernameStyle, fontWeight: 300 }} className="break-words italic">
                <Tooltip content="Right-click to reply" side="top">
                  <span
                    style={{ fontWeight: 700 }}
                    className="cursor-pointer hover:underline inline-block"
                    onClick={(e) => {
                      const userId = clickUserId;
                      const displayName = parsed.tags.get('display-name') || parsed.username;
                      if (userId && onUsernameClick) {
                        onUsernameClick(
                          userId,
                          parsed.username,
                          displayName,
                          parsed.color,
                          parsed.badges,
                          e,
                          clickProvider
                        );
                      }
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      const messageId = parsed.tags.get('id');
                      if (messageId && onUsernameRightClick) {
                        onUsernameRightClick(messageId, parsed.username);
                      }
                    }}
                  >
                    {renderedName}
                    {broadcasterType === 'partner' && (
                      <svg
                        className="w-3.5 h-3.5 inline-block flex-shrink-0 ml-1"
                        viewBox="0 0 16 16"
                        fill="#9146FF"
                        style={{ verticalAlign: 'middle' }}
                      >
                        <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd"></path>
                      </svg>
                    )}
                  </span>
                </Tooltip>
                {' '}{replyMentionNode}{giantPluck ? giantPluck.inline : renderContent(contentWithEmotes)}
              </span>
            ) : (
              // Regular messages: username in color, content in default color
              <>
                <StyledChatName
                  name={renderedName}
                  nameTextStyle={usernameStyle}
                  nameStyle={nameStyle}
                  separator={nameSeparator}
                  accentColor={prefixAccentColor}
                  interactive
                  onClick={(e) => {
                    const userId = clickUserId;
                    const displayName = parsed.tags.get('display-name') || parsed.username;
                    if (userId && onUsernameClick) {
                      onUsernameClick(
                        userId,
                        parsed.username,
                        displayName,
                        parsed.color,
                        parsed.badges,
                        e,
                          clickProvider
                      );
                    }
                  }}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    const messageId = parsed.tags.get('id');
                    if (messageId && onUsernameRightClick) {
                      onUsernameRightClick(messageId, parsed.username);
                    }
                  }}
                  badge={broadcasterType === 'partner' ? (
                    <svg
                      className="w-3.5 h-3.5 inline-block flex-shrink-0 ml-1"
                      viewBox="0 0 16 16"
                      fill="#9146FF"
                      style={{ verticalAlign: 'middle' }}
                    >
                      <path fillRule="evenodd" d="M12.5 3.5 8 2 3.5 3.5 2 8l1.5 4.5L8 14l4.5-1.5L14 8l-1.5-4.5ZM7 11l4.5-4.5L10 5 7 8 5.5 6.5 4 8l3 3Z" clipRule="evenodd"></path>
                    </svg>
                  ) : undefined}
                />
                <span style={{ fontWeight: 'var(--chat-body-weight, 300)' }} className="text-textPrimary break-words">
                  <span
                    style={
                      moderationContext && (chatDesign?.deleted_message_style ?? 'strikethrough') === 'strikethrough'
                        ? { textDecoration: 'line-through' }
                        : undefined
                    }
                  >
                    {' '}{replyMentionNode}{giantPluck ? giantPluck.inline : renderContent(contentWithEmotes)}
                  </span>
                  {redemptionCost && (
                    <span className="inline-flex items-center gap-0.5 ml-1 align-middle text-highlight-cyan/90 font-medium">
                      {redemptionPointsIcon ? (
                        <img
                          src={redemptionPointsIcon}
                          alt=""
                          className="w-3.5 h-3.5 inline-block align-middle rounded-full object-cover"
                          onError={(e) => { e.currentTarget.style.display = 'none'; }}
                        />
                      ) : (
                        <ChannelPointsIcon size={14} className="text-highlight-cyan/90" />
                      )}
                      {Number(redemptionCost).toLocaleString()}
                    </span>
                  )}
                  {moderationContext && (chatDesign?.deleted_message_style ?? 'strikethrough') === 'strikethrough' && (
                    <span className="ml-1.5 text-xs text-error/70 font-medium">
                      {moderationContext.type === 'timeout'
                        ? `[timed out for ${moderationContext.duration}s]`
                        : moderationContext.type === 'ban'
                          ? '[banned]'
                          : '[deleted by mod]'}
                    </span>
                  )}
                  {showRepeatCount && (
                    <Tooltip content={repeatTooltip} side="top">
                      <span
                        className="ml-1.5 text-xs font-semibold tabular-nums cursor-default"
                        style={{ color: repeatSettings?.color || 'var(--color-text-secondary)' }}
                      >
                        x{repeat.count}
                      </span>
                    </Tooltip>
                  )}
                </span>
              </>
            )}
          </span>
        </div>
      </div>
      {/* Gigantified emote: the plucked last emote at 4x, block-level below
          the body (same tier as link previews / SongCard). fontSize matches
          the body so the em-based giant height tracks the chat font setting. */}
      {giantPluck?.giant && (
        <div className={`mt-1 flex ${giantJustify}`} style={{ fontSize: `${chatDesign?.font_size ?? 14}px` }}>
          {giantPluck.giant}
        </div>
      )}
      {/* Inline link preview cards. Trusted links auto-expand; untrusted links
          render a click-to-load chip (trusted={false}). showChip (the no-preview
          fallback link chip) applies only to trusted links in clean mode, where
          the inline link is hidden so the card/chip is the sole representation. */}
      {linkPreviewItems.length > 0 && !streamerModeActive && (
        <div className="flex flex-col items-start gap-1">
          {linkPreviewItems.map((it) => (
            <LinkPreviewCard
              key={it.url}
              url={it.url}
              trusted={it.trusted}
              showChip={it.trusted && !keepInlineLink}
            />
          ))}
        </div>
      )}
      {/* Inline quick-actions ON the message itself (top-right) — NOT the mod
          menu: Copy for everyone + Pin for mods (when the pin style includes
          inline). Hover-revealed; data-no-drag so they never start a mod drag. */}
      {hoverArmed && (onMessageCopy || showInlinePin) && broadcasterId && (
        <div
          data-no-drag="true"
          className="absolute top-1 right-2 z-[50] opacity-0 group-hover:opacity-100 transition-opacity flex items-center gap-0.5 p-0.5 rounded-lg bg-tertiary/90 backdrop-blur-sm border border-white/10 shadow-[0_4px_12px_rgba(0,0,0,0.4)]"
        >
          {showInlinePin && thisMessageId && parsed.provider === 'twitch' && (
            <Tooltip content={isThisPinned ? 'Unpin message' : 'Pin message'} side="left">
              <button
                onClick={async (e) => {
                  e.preventDefault();
                  if (!thisMessageId) return;
                  await attempt(async () => {
                    const invoke = await loadInvoke();
                    if (isThisPinned) {
                      await invoke('unpin_chat_message', { broadcasterId, messageId: thisMessageId });
                      useAppStore.getState().addToast('Unpinned message', 'success');
                    } else {
                      await invoke('pin_chat_message', { broadcasterId, messageId: thisMessageId, durationSeconds: null });
                      useAppStore.getState().addToast('Pinned message', 'success');
                    }
                    usePinStore.getState().requestRefresh();
                  }, (err) => {
                    Logger.error('[ChatMessage] Pin/unpin failed:', err);
                    useAppStore.getState().addToast(isThisPinned ? "Couldn't unpin that message" : "Couldn't pin that message", 'error');
                  });
                }}
                className={`p-1.5 rounded-md transition-colors ${isThisPinned ? 'text-accent hover:text-error hover:bg-error/15' : 'text-white/50 hover:text-accent hover:bg-accent/15'}`}
              >
                {isThisPinned ? (
                  <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="2" x2="22" y1="2" y2="22"/><line x1="12" x2="12" y1="17" y2="22"/><path d="M9 9v1.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V17h12"/><path d="M15 9.34V6h1a2 2 0 0 0 0-4H7.89"/></svg>
                ) : (
                  <svg className="w-3.5 h-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><line x1="12" x2="12" y1="17" y2="22"/><path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"/></svg>
                )}
              </button>
            </Tooltip>
          )}
          {onMessageCopy && (
            <Tooltip content="Copy message" side="left">
              <button
                onClick={(e) => { e.preventDefault(); onMessageCopy(parsed.content); }}
                className="p-1.5 rounded-md text-white/50 hover:text-white hover:bg-surface-hover transition-colors"
              >
                <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z" /></svg>
              </button>
            </Tooltip>
          )}
        </div>
      )}

      {/* Moderation menu: floats ABOVE the message (7TV-style), mod-only. The
          punitive actions live here, away from the message text; Copy + Pin are
          inline above. */}
      {hoverArmed && isModerator && showModButtons && broadcasterId && (
        <div
          data-no-drag="true"
          className="absolute bottom-full right-2 mb-0.5 opacity-0 group-hover:opacity-100 transition-[opacity,transform] duration-200 flex items-center gap-0.5 p-0.5 backdrop-blur-md border border-white/10 shadow-[0_8px_24px_rgba(0,0,0,0.55)] rounded-xl overflow-visible z-[50] translate-y-1 group-hover:translate-y-0"
          // Themed (was hardcoded zinc-900); transition stays scoped so the
          // backdrop-filter itself is never animated.
          style={{ backgroundColor: 'color-mix(in srgb, var(--color-background-tertiary) 95%, transparent)' }}
        >
              {/* Delete Message (Twitch + Kick). */}
              <Tooltip content="Delete Message" side="top">
                <button
                  onClick={(e) => {
                    e.preventDefault();
                    const msgId = parsed.tags.get('id');
                    if (msgId) void modDelete(msgId);
                  }}
                  className="p-2 rounded-lg hover:bg-error/15 text-white/50 hover:text-error transition-colors"
                >
                  <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><polyline points="3 6 5 6 21 6"></polyline><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path></svg>
                </button>
              </Tooltip>
              <div className="w-px h-5 bg-white/10" />

              {/* Quick Timeout (10m) */}
              <div className="relative flex group/timeout">
            <Tooltip content="Timeout User (10m) — shift-click to add a reason" side="top">
              <button
                onClick={(e) => {
                  e.preventDefault();
                  if (e.shiftKey) { prefillModCommand(600); return; }
                  void modBan(600);
                }}
                className="p-2 rounded-lg hover:bg-warning/20 text-white/50 hover:text-warning transition-colors"
              >
                <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10"></circle><polyline points="12 6 12 12 16 14"></polyline></svg>
              </button>
            </Tooltip>
            
            {/* Timeout Dropdown */}
            <div className="absolute bottom-full left-1/2 -translate-x-1/2 opacity-0 pointer-events-none group-hover/timeout:opacity-100 group-hover/timeout:pointer-events-auto transition-opacity px-2 pb-1.5">
              <div className="flex bg-tertiary border border-white/10 rounded-md shadow-xl overflow-hidden">
                {timeoutPresets.map(opt => (
                  <button
                    key={opt.val}
                    className="px-2 py-1 text-[10px] font-bold text-white/70 hover:text-warning hover:bg-white/10 transition-colors border-r border-white/5 last:border-0"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      if (e.shiftKey) { prefillModCommand(opt.val); return; }
                      void modBan(opt.val);
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </div>
          </div>

          <div className="w-px h-5 bg-white/10" />

          {/* Quick Ban */}
          <Tooltip content="Ban User — shift-click to add a reason" side="top">
            <button
              onClick={(e) => {
                e.preventDefault();
                if (e.shiftKey) { prefillModCommand(null); return; }
                void modBan(null);
              }}
              className="p-2 rounded-lg hover:bg-error/15 text-white/50 hover:text-error transition-colors"
            >
              <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>
            </button>
          </Tooltip>
        </div>
      )}
    </div>
  );
}, chatMessageAreEqual);

export default ChatMessage;
