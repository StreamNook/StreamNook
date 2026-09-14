import React, { useRef, useEffect, useCallback, useMemo, memo } from 'react';
import ChatMessage from './ChatMessage';
import { EmoteSet } from '../services/emoteService';
import { BackendChatMessage } from '../services/twitchChat';
import { ModerationContext } from '../hooks/useTwitchChat';
import { useAppStore } from '../stores/AppStore';
import { useStreamerMode } from '../utils/streamerMode';
import { useChatUserStore } from '../stores/chatUserStore';
import { ProviderLogo } from './ProviderLogo';
import type { ProviderId } from '../types/providers';

/**
 * Per-row wrapper carrying the native-virtualization styles, with one
 * exception: a row whose sender has an animated Atmosphere opts OUT of
 * `content-visibility: auto`.
 *
 * Atmosphere members render a perpetually-composited aurora layer behind their
 * message (`.sn-aurora-*` in globals.css — an infinite `transform` animation
 * with `will-change`). Under `content-visibility: auto`, WebView2/Chromium
 * fails to invalidate that composited layer as the row scrolls, stranding a
 * stale paint ghost of the entire row. A fresh ghost is left behind with every
 * new message that scrolls the row upward, so a single message ends up looking
 * like it was sent many times over. Painting these (few) rows normally lets the
 * compositor track their scroll correctly. Plain rows have no composited layer,
 * never ghost, and keep full virtualization.
 */
interface MessageRowProps {
  messageId: string | null;
  userId: string | undefined;
  isModFocus: boolean;
  intrinsicSizeCSS: string;
  sourceProvider: ProviderId | undefined;
  sourceLogoSize: number;
  message: string | BackendChatMessage;
  isHighlighted: boolean;
  moderationContext: ModerationContext | null;
  emotes: EmoteSet | null;
  isModerator?: boolean;
  broadcasterId?: string;
  onUsernameClick: ChatMessageListProps['onUsernameClick'];
  onReplyClick: ChatMessageListProps['onReplyClick'];
  onMessageCopy?: ChatMessageListProps['onMessageCopy'];
  onEmoteRightClick: ChatMessageListProps['onEmoteRightClick'];
  onUsernameRightClick: ChatMessageListProps['onUsernameRightClick'];
  onBadgeClick: ChatMessageListProps['onBadgeClick'];
}

// Mirrors chatMessageAreEqual's rules for the shared props, so the row-level
// memo never suppresses a repaint the inner comparator would allow. The
// message rule replicates its string branch exactly: a string on EITHER side
// re-renders (own-echo upgrade, repaintOwnBadges/Color and the id-stamp all
// swap same-id string values in place), and reference-unequal objects are
// equal only when both ids exist and match. Callbacks are ignored, same
// contract as chatMessageAreEqual.
const messageRowAreEqual = (prev: MessageRowProps, next: MessageRowProps): boolean => {
  if (prev.messageId !== next.messageId) return false;
  if (prev.userId !== next.userId) return false;
  if (prev.isModFocus !== next.isModFocus) return false;
  if (prev.intrinsicSizeCSS !== next.intrinsicSizeCSS) return false;
  if (prev.sourceProvider !== next.sourceProvider) return false;
  if (prev.sourceLogoSize !== next.sourceLogoSize) return false;
  if (prev.message !== next.message) {
    if (typeof prev.message === 'string' || typeof next.message === 'string') return false;
    if (!prev.message.id || !next.message.id) return false;
    if (prev.message.id !== next.message.id) return false;
  }
  if (prev.isHighlighted !== next.isHighlighted) return false;
  if (prev.moderationContext?.type !== next.moderationContext?.type ||
      prev.moderationContext?.duration !== next.moderationContext?.duration) return false;
  if (prev.emotes !== next.emotes) return false;
  if (prev.isModerator !== next.isModerator) return false;
  if (prev.broadcasterId !== next.broadcasterId) return false;
  return true;
};

const MessageRow = memo(function MessageRow({
  messageId,
  userId,
  isModFocus,
  intrinsicSizeCSS,
  sourceProvider,
  sourceLogoSize,
  message,
  isHighlighted,
  moderationContext,
  emotes,
  isModerator,
  broadcasterId,
  onUsernameClick,
  onReplyClick,
  onMessageCopy,
  onEmoteRightClick,
  onUsernameRightClick,
  onBadgeClick,
}: MessageRowProps) {
  const hasAtmosphere = useChatUserStore((s) => {
    if (!userId) return false;
    const u = s.users.get(userId);
    // Cologne rows carry the same animated composited wash as an Atmosphere, so
    // they need the same always-paint treatment to dodge the ghost bug.
    return !!(u?.atmosphereId || u?.cologne);
  });
  const style = useMemo(
    () => ({
      // Native virtualization for normal rows; atmosphere rows paint always
      // to dodge the content-visibility compositing-ghost bug (see above).
      contentVisibility: hasAtmosphere ? ('visible' as const) : ('auto' as const),
      // Off-screen size hint, computed per-user from font size, spacing, and
      // whether timestamps are on. Ignored when content-visibility is visible.
      containIntrinsicBlockSize: hasAtmosphere ? undefined : intrinsicSizeCSS,
    }),
    [hasAtmosphere, intrinsicSizeCSS],
  );
  const chatMessageEl = (
    <ChatMessage
      message={message}
      onUsernameClick={onUsernameClick}
      onReplyClick={onReplyClick}
      onMessageCopy={onMessageCopy}
      isHighlighted={isHighlighted}
      moderationContext={moderationContext}
      onEmoteRightClick={onEmoteRightClick}
      onUsernameRightClick={onUsernameRightClick}
      onBadgeClick={onBadgeClick}
      emotes={emotes}
      isModerator={isModerator}
      broadcasterId={broadcasterId}
    />
  );
  return (
    <div
      data-message-id={messageId || undefined}
      className={`chat-message-row${isModFocus ? ' is-mod-focus' : ''}${
        typeof message !== 'string' && message.metadata?.from_backfill ? ' is-backfill' : ''
      }`}
      style={style}
    >
      {sourceProvider ? (
        <div className="flex items-center gap-1.5 pl-1">
          <ProviderLogo provider={sourceProvider} size={sourceLogoSize} />
          <div className="min-w-0 flex-1">{chatMessageEl}</div>
        </div>
      ) : (
        chatMessageEl
      )}
    </div>
  );
}, messageRowAreEqual);

interface ChatMessageListProps {
  messages: (string | BackendChatMessage)[];
  /**
   * Bump this whenever the feed changed. It is not read in the body — it exists
   * purely so this component's `memo` has a reliable change signal.
   *
   * Keep it. The store now writes a fresh array on every change (copy-on-write,
   * see chatConnectionStore), so `messages` identity is a valid signal too;
   * the token is the channel-wide one and also covers moderation-mark and
   * meta changes that are not list props. Use the channel's `renderToken`
   * from ChannelChatSnapshot.
   */
  renderToken: number;
  isPaused: boolean;
  onScroll: (distanceToBottom: number, isUserScroll: boolean) => void;
  // Fired the instant a real user scroll-up gesture is detected (wheel/touch/
  // keyboard), BEFORE any scroll event or distance threshold. This is the
  // reliable pause signal in fast chats — it never depends on out-racing the
  // auto-scroll snap-back, and it is naturally immune to layout jolts (a tall
  // message moves the scrollbar but never emits a wheel event).
  onPauseIntent?: () => void;
  onUsernameClick: (
    userId: string,
    username: string,
    displayName: string,
    color: string,
    badges: Array<{ key: string; info: Record<string, unknown> }>,
    event: React.MouseEvent
  ) => void;
  onReplyClick: (parentMsgId: string) => void;
  onMessageCopy?: (content: string) => void;
  onEmoteRightClick: (emoteName: string) => void;
  onUsernameRightClick: (messageId: string, username: string) => void;
  onBadgeClick: (badgeKey: string, badgeInfo: Record<string, unknown>) => void;
  highlightedMessageId: string | null;
  /** Message id with the persistent keyboard-moderation focus ring (or null). */
  modFocusId?: string | null;
  deletedMessageIds: Set<string>;
  // Messages whose IDs are in this set are filtered out entirely (rendered
  // nothing). Used by /clearmessages for visual-only chat clears, distinct
  // from `deletedMessageIds` which renders moderation context (strikethrough).
  hiddenMessageIds?: Set<string>;
  clearedUserContexts: Map<string, { context: ModerationContext; affectedMessageIds: Set<string> }>;
  emotes: EmoteSet | null;
  getMessageId: (message: string | BackendChatMessage) => string | null;
  isModerator?: boolean;
  broadcasterId?: string;
  /** Blended view: mark each row with its source platform (a provider-colored
   *  left stripe), so a merged multi-source feed shows where each message is from.
   *  Off everywhere else, so normal single-source chat is untouched. */
  showSource?: boolean;
}

/**
 * ChatMessageList - Scrollable chat message container
 * 
 * Uses CSS `content-visibility: auto` for browser-native virtualization.
 * No JavaScript height estimation or measurement - the browser handles everything.
 * 
 * Benefits:
 * - Zero jitter: Heights are never "estimated" or "corrected"
 * - Stable scrolling: No ResizeObserver callbacks causing layout shifts
 * - Simpler architecture: Just render messages, browser does the rest
 */
const ChatMessageList = memo(function ChatMessageList({
  messages,
  isPaused,
  onScroll,
  onPauseIntent,
  onUsernameClick,
  onReplyClick,
  onMessageCopy,
  onEmoteRightClick,
  onUsernameRightClick,
  onBadgeClick,
  highlightedMessageId,
  modFocusId,
  hiddenMessageIds,
  deletedMessageIds,
  clearedUserContexts,
  emotes,
  getMessageId,
  isModerator,
  broadcasterId,
  showSource,
}: ChatMessageListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  // Inner messages-wrapper div. We observe its size with a ResizeObserver so
  // that any height change (badge/emote/cosmetic images settling in after a
  // new message's first paint, content-visibility resolving to a slightly
  // different actual size, etc.) re-pins the scroll to bottom while the user
  // was already there. Without this, every new message left a brief upward
  // shimmer for the time between the first scrollTop=scrollHeight call and
  // when async content finished sizing.
  const contentRef = useRef<HTMLDivElement>(null);
  const isScrollingProgrammatically = useRef(false);
  const wasAtBottomRef = useRef(true); // Track if we were at bottom BEFORE new messages
  const prevMessageCountRef = useRef(0); // Track previous message count for channel switch detection
  // Resume-glide animation state (see scrollToBottom). While a glide is in
  // flight this is true, and the auto-scroll re-pin sites below stand down so
  // freshly-arriving messages don't instant-jump and stutter the smooth scroll
  // back to the live bottom. resumeRafRef holds the in-flight frame so a new
  // resume (or unmount) can cancel it.
  const isResumeAnimatingRef = useRef(false);
  const resumeRafRef = useRef<number | null>(null);

  // Per-row CSS `contain-intrinsic-block-size` placeholder size. The browser
  // uses this for off-screen messages (content-visibility: auto). Picking a
  // value close to the real rendered height prevents a layout shift on the
  // FIRST render of each message — the prior hardcoded 50px was tuned for
  // single-line messages, which was way off when timestamps add a second line.
  // The `auto` keyword still lets the browser remember the actual size after
  // first render, so this is just the initial-paint guess.
  const chatDesign = useAppStore((s) => s.settings.chat_design);
  const intrinsicSizeCSS = useMemo(() => {
    const fontSize = chatDesign?.font_size ?? 14;
    const messageSpacing = chatDesign?.message_spacing ?? 2;
    // One content line ≈ font_size * 1.5 (browser default leading) plus
    // top + bottom padding (each = max(4, spacing / 2)) and the optional
    // 1px divider border underneath.
    const padding = Math.max(4, messageSpacing / 2) * 2;
    const contentLine = Math.round(fontSize * 1.5);
    // Timestamp row is a 10px text line with line-height tight (~1.25) plus
    // mb-0.5 (~2px). Total ≈ 14-15px when enabled.
    const timestampHeight = chatDesign?.show_timestamps ? 15 : 0;
    const total = contentLine + timestampHeight + padding + 1; // +1 for divider
    return `auto ${total}px`;
  }, [chatDesign?.font_size, chatDesign?.message_spacing, chatDesign?.show_timestamps]);

  // Source provider logo (blended feed): a touch larger than the text and scaling
  // with the chat font size — like inline emotes — so it stays legible as the user
  // sizes the chat up.
  const sourceLogoSize = Math.round((chatDesign?.font_size ?? 14) * 1.2);
  
  // Track if user explicitly scrolled UP via wheel (negative deltaY = scroll up)
  // This is the ONLY reliable way to detect user scroll intent
  // Flag persists until user scrolls DOWN or reaches bottom
  const userScrolledUpRef = useRef(false);
  const lastTouchY = useRef(0);

  // Handle wheel events - detect scroll-up intent directly from deltaY.
  // The wheel gesture is the single source of truth for "the user wants to
  // scroll up" — it fires only on real input, so we act on it immediately
  // instead of waiting for a scroll event + distance threshold that the
  // auto-scroll would keep stomping on in a fast chat.
  const handleWheel = useCallback((e: React.WheelEvent) => {
    if (e.deltaY < 0) {
      // Scrolling UP (away from bottom). Setting the ref here also halts every
      // programmatic re-pin below on the very next frame, so the snap-back
      // cannot yank the view back down before React commits the paused state.
      userScrolledUpRef.current = true;
      onPauseIntent?.();
    } else if (e.deltaY > 0) {
      // Scrolling DOWN (toward bottom) - clear the flag
      userScrolledUpRef.current = false;
    }
  }, [onPauseIntent]);

  // Handle touch scrolling - track direction via Y position change
  const handleTouchStart = useCallback((e: React.TouchEvent) => {
    lastTouchY.current = e.touches[0].clientY;
  }, []);

  // Touch is the INVERSE of wheel, and reading it the wheel way is what made
  // touch chat thrash: dragging the finger DOWN drags content down with it,
  // revealing OLDER messages (scrollTop decreases) — that is the scroll-up
  // intent that must pause. Dragging UP chases the newest messages, which is
  // the return-to-live direction. Inverted, every "let me read back" gesture
  // cleared the pause and the auto-scroll yanked the view straight back down.
  const handleTouchMove = useCallback((e: React.TouchEvent) => {
    const currentY = e.touches[0].clientY;
    const fingerDelta = currentY - lastTouchY.current; // positive = finger moved down
    lastTouchY.current = currentY;

    if (fingerDelta > 0) {
      // Finger down: revealing older messages (away from the live bottom).
      userScrolledUpRef.current = true;
      onPauseIntent?.();
    } else if (fingerDelta < 0) {
      // Finger up: heading back toward the live bottom - clear the flag.
      userScrolledUpRef.current = false;
    }
  }, [onPauseIntent]);

  // ResizeObserver: re-pin to bottom whenever the inner messages-wrapper
  // grows (badge/emote images settling, paint shaders sizing, etc.). This is
  // the secondary safety net beyond the explicit auto-scroll-on-new-message
  // effect below — it catches any height change the explicit path missed,
  // and is the actual fix for the "timestamp shimmer" where a new row's
  // height grows a few pixels after first paint.
  useEffect(() => {
    if (!contentRef.current) return;
    const target = contentRef.current;
    const ro = new ResizeObserver(() => {
      if (!containerRef.current) return;
      if (isPaused) return;
      // A resume glide is driving the scroll itself; an instant re-pin here
      // would jump past the animation and kill the smoothness.
      if (isResumeAnimatingRef.current) return;
      // Bail the instant the user expresses scroll-up intent, even before the
      // paused state has committed — otherwise a height change landing in that
      // window would re-pin to bottom and fight the pause.
      if (userScrolledUpRef.current) return;
      // Only re-pin when the user was already at bottom — never YANK them
      // down if they've scrolled up to read history.
      if (!wasAtBottomRef.current) return;
      isScrollingProgrammatically.current = true;
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
      // Brief lock so the synthetic scroll event the browser emits doesn't
      // get interpreted as a "user scrolled away" event.
      setTimeout(() => {
        isScrollingProgrammatically.current = false;
      }, 60);
    });
    ro.observe(target);
    return () => ro.disconnect();
  }, [isPaused]);

  // Auto-scroll to bottom when new messages arrive
  // SIMPLE RULE: If not paused, always scroll to bottom
  const lastMessageId = messages.length > 0 ? getMessageId(messages[messages.length - 1]) : null;

  useEffect(() => {
    if (!containerRef.current) return;
    
    const prevCount = prevMessageCountRef.current;
    const currentCount = messages.length;
    
    // Detect channel switch: messages went from 0 to N (bulk load from IVR history)
    const isChannelLoad = prevCount === 0 && currentCount > 0;
    
    // Update ref for next render
    prevMessageCountRef.current = currentCount;
    
    // If paused (and not a channel load), don't auto-scroll
    if (!isChannelLoad && isPaused) return;
    // Same for fresh up-intent that hasn't propagated to `isPaused` yet — the
    // wheel/touch handler sets this synchronously, so honoring it here closes
    // the one-frame gap where a fast-arriving message could snap the view back
    // down before the pause commits. (A channel load always re-pins.)
    if (!isChannelLoad && userScrolledUpRef.current) return;
    // A resume glide owns the scroll position until it lands; let it run
    // rather than instant-jumping on every message and stuttering it.
    if (isResumeAnimatingRef.current) return;

    // NOT PAUSED: Always scroll to bottom
    // Use double-scroll pattern to ensure we catch the final height after content-visibility resolves

    // First scroll: immediate RAF to catch initial render
    requestAnimationFrame(() => {
      if (!containerRef.current) return;
      if (!isChannelLoad && userScrolledUpRef.current) return;
      if (isResumeAnimatingRef.current) return;
      isScrollingProgrammatically.current = true;
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
      wasAtBottomRef.current = true;
    });

    // Second scroll: short delay to catch content-visibility final height calculation
    // This fixes the issue where new messages appear partially behind the input box
    setTimeout(() => {
      if (!containerRef.current || isPaused) return;
      if (!isChannelLoad && userScrolledUpRef.current) return;
      if (isResumeAnimatingRef.current) return;
      isScrollingProgrammatically.current = true;
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
      wasAtBottomRef.current = true;
      
      // Extend lock to 150ms to swallow any layout-shift scroll events
      setTimeout(() => {
        isScrollingProgrammatically.current = false;
      }, 150);
    }, 50);
    
    // For channel loads, do a third scroll after longer delay
    if (isChannelLoad) {
      setTimeout(() => {
        if (containerRef.current) {
          isScrollingProgrammatically.current = true;
          containerRef.current.scrollTop = containerRef.current.scrollHeight;
          setTimeout(() => {
            isScrollingProgrammatically.current = false;
          }, 150);
        }
      }, 150);
    }
  }, [lastMessageId, isPaused, messages.length]); // Track lastMessageId to handle buffer trimming updates

  // Handle scroll events - this updates wasAtBottomRef for the NEXT message arrival
  const handleScroll = useCallback(() => {
    if (!containerRef.current) return;
    
    const { scrollTop, scrollHeight, clientHeight } = containerRef.current;
    const distanceToBottom = scrollHeight - clientHeight - scrollTop;
    
    // Update "was at bottom" state (used by the NEXT render's auto-scroll check)
    // Only update if this wasn't a programmatic scroll
    if (!isScrollingProgrammatically.current) {
      const atBottom = distanceToBottom < 100;
      wasAtBottomRef.current = atBottom;
      
      // Only report as user scroll if:
      // 1. User explicitly scrolled UP via wheel/touch (userScrolledUpRef is true)
      // 2. AND we're actually away from bottom now
      const isUserScroll = userScrolledUpRef.current && distanceToBottom > 50;
      
      // Clear flag only when reaching bottom (user scrolled back down)
      if (atBottom) {
        userScrolledUpRef.current = false;
      }
      
      onScroll(distanceToBottom, isUserScroll);
    } else {
      // For programmatic scrolls, we ARE at bottom.
      wasAtBottomRef.current = true;
      // Do NOT clear userScrolledUpRef here. The auto-scroll fires a synthetic
      // scroll event on every new message, and in a fast chat that ran almost
      // continuously — wiping the flag here is what erased the user's scroll-up
      // intent before it could ever pause. Intent is only cleared by a genuine
      // downward gesture or by actually reaching the bottom (handled above).
    }
  }, [onScroll]);

  // Scroll to bottom (for the Resume button + auto-resume). Smooth by default:
  // a brisk eased glide back to the live bottom rather than an abrupt snap.
  // Auto-scroll on new messages (the other scrollTop sites above) intentionally
  // stays instant; animating those would fight itself in fast chats.
  const smoothScrollOnResume =
    useAppStore((s) => s.settings.chat_render?.smooth_scroll_on_resume) ?? true;

  // Cancel any in-flight resume glide and release its guard.
  const cancelResumeAnimation = useCallback(() => {
    if (resumeRafRef.current !== null) {
      cancelAnimationFrame(resumeRafRef.current);
      resumeRafRef.current = null;
    }
    isResumeAnimatingRef.current = false;
  }, []);

  const scrollToBottom = useCallback(() => {
    const el = containerRef.current;
    if (!el) return;
    // A new resume supersedes any glide already running.
    cancelResumeAnimation();
    isScrollingProgrammatically.current = true;
    wasAtBottomRef.current = true;
    // Resuming is a deliberate return to live — clear up-intent so the
    // auto-scroll guards re-engage cleanly on the next message.
    userScrolledUpRef.current = false;

    const liveBottom = () => el.scrollHeight - el.clientHeight;
    const start = el.scrollTop;
    const distance = liveBottom() - start;

    // Snap instantly for tiny gaps (e.g. the auto-resume that fires when you've
    // manually scrolled almost to the bottom) — a 20px floaty glide reads worse
    // than just being there. Also the fallback when smooth is toggled off.
    if (!smoothScrollOnResume || distance < 40) {
      el.scrollTop = el.scrollHeight;
      requestAnimationFrame(() => {
        isScrollingProgrammatically.current = false;
      });
      return;
    }

    // Brisk, distance-proportional duration (200–420ms) with an ease-out
    // landing — fast off the line, decelerating gently into the bottom.
    const duration = Math.min(420, Math.max(200, distance * 0.4));
    const startTime = performance.now();
    const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3);

    isResumeAnimatingRef.current = true;
    const step = (now: number) => {
      const node = containerRef.current;
      // Container gone, or the user grabbed the wheel mid-glide — abort and let
      // the (re)pause path take over cleanly.
      if (!node || userScrolledUpRef.current) {
        cancelResumeAnimation();
        isScrollingProgrammatically.current = false;
        return;
      }
      const t = Math.min(1, (now - startTime) / duration);
      // Re-read the target every frame so the glide tracks messages that arrive
      // mid-animation and still lands exactly on the newest one.
      const target = node.scrollHeight - node.clientHeight;
      node.scrollTop = start + (target - start) * easeOutCubic(t);
      if (t < 1) {
        resumeRafRef.current = requestAnimationFrame(step);
        return;
      }
      // Landed: pin to the true live bottom and hand control back.
      node.scrollTop = node.scrollHeight;
      resumeRafRef.current = null;
      isResumeAnimatingRef.current = false;
      requestAnimationFrame(() => {
        isScrollingProgrammatically.current = false;
      });
    };
    resumeRafRef.current = requestAnimationFrame(step);
  }, [smoothScrollOnResume, cancelResumeAnimation]);

  // Cancel a glide in flight if the list unmounts.
  useEffect(() => () => cancelResumeAnimation(), [cancelResumeAnimation]);

  // Expose scrollToBottom to parent via ref callback pattern
  useEffect(() => {
    // This makes the scroll function available outside
    (window as Window & typeof globalThis & { __chatScrollToBottom?: () => void }).__chatScrollToBottom = scrollToBottom;
    return () => {
      delete (window as Window & typeof globalThis & { __chatScrollToBottom?: () => void }).__chatScrollToBottom;
    };
  }, [scrollToBottom]);

  const emoteScale = chatDesign?.emote_scale ?? 1;
  const emoteMargin = chatDesign?.emote_margin ?? 0.125;
  const deletedStyle = chatDesign?.deleted_message_style ?? 'strikethrough';
  // Streamer mode hides restricted (low-trust) users' rows: a ban evader's
  // message must not appear on stream while a mod reviews it.
  const streamerModeActive = useStreamerMode((st) => st.active);
  const hideSharedChat = chatDesign?.hide_shared_chat ?? false;

  // Tracks ids already rendered THIS pass so a duplicate id in the message
  // array can never produce two children with the same React key. Duplicate
  // keys break reconciliation: React duplicates/omits the colliding rows and
  // orphans their DOM on every subsequent render (each new message), which is
  // both the visual "my message stacked many times" symptom and a steadily
  // growing memory leak. The store tries hard to dedupe (seenMessageIds), but
  // own-message reconciliation leaves a gap (a sent message's real id is never
  // added to seenMessageIds, so a backfill/echo can re-add it). This is the
  // last-line guarantee that the render layer is always key-safe regardless.
  // Fresh per render.
  const renderedIds = new Set<string>();

  return (
    <div
      ref={containerRef}
      className="h-full overflow-y-auto overflow-x-hidden scrollbar-thin"
      onScroll={handleScroll}
      onWheel={handleWheel}
      onTouchStart={handleTouchStart}
      onTouchMove={handleTouchMove}
      style={{
        overflowAnchor: 'auto',
        '--sn-emote-scale': emoteScale,
        '--sn-emote-margin': `${emoteMargin}rem`,
        '--sn-backfill-opacity': Math.max(0.3, Math.min(1, (chatDesign?.backfill_opacity ?? 100) / 100)),
      } as React.CSSProperties}
    >
      {/* Messages container with native virtualization - pt-10 for header */}
      <div ref={contentRef} className={`flex flex-col min-h-full justify-end pt-10${chatDesign?.alternating_backgrounds ? ' chat-striped' : ''}`}>
        {messages.map((message, index) => {
          const messageId = getMessageId(message);

          // Render each id at most once. If the array somehow holds a duplicate
          // (e.g. an own message present both as the stamped optimistic copy and
          // a backfilled/echoed copy), drop the later one so React never sees a
          // duplicate key. See renderedIds note above.
          if (messageId) {
            if (renderedIds.has(messageId)) return null;
            renderedIds.add(messageId);
          }

          // /clearmessages — skip rendering entirely for messages snapshotted
          // into the locally-hidden set. Distinct from the deleted/moderated
          // path below, which renders strikethrough chrome.
          if (messageId && hiddenMessageIds?.has(messageId)) {
            return null;
          }

          // Sender user-id — used both for cleared-user moderation context and
          // for the per-row Atmosphere lookup in MessageRow below.
          const userId = typeof message !== 'string'
            ? message.user_id
            : message.match(/user-id=([^;]+)/)?.[1];

          // Check if message is deleted/moderated
          let moderationContext: ModerationContext | null = null;

          if (messageId && deletedMessageIds.has(messageId)) {
            // Single message deleted by mod
            moderationContext = { type: 'deleted' };
          } else if (messageId) {
            if (userId && clearedUserContexts.has(userId)) {
              const entry = clearedUserContexts.get(userId)!;
              if (entry.affectedMessageIds.has(messageId)) {
                moderationContext = entry.context;
              }
            }
          }

          // Deleted message style: 'hidden' suppresses the row entirely.
          // Other styles (strikethrough/dimmed/keep) fall through to ChatMessage.
          if (moderationContext && deletedStyle === 'hidden') {
            return null;
          }
          if (
            streamerModeActive &&
            typeof message !== 'string' &&
            message.metadata?.suspicious === 'restricted'
          ) {
            return null;
          }
          // Hide shared-chat-flagged messages if the user opted in.
          if (hideSharedChat && typeof message !== 'string') {
            const srcRoom = message.tags?.['source-room-id'] as string | undefined;
            const curRoom = message.tags?.['room-id'] as string | undefined;
            if (srcRoom && curRoom && srcRoom !== curRoom) {
              return null;
            }
          }
          // 'keep' style nukes the moderation context so ChatMessage renders
          // as if nothing happened.
          if (moderationContext && deletedStyle === 'keep') {
            moderationContext = null;
          }

          // Blended feed: prefix each message with its source platform's logo so
          // a merged multi-source feed is readable at a glance. Only when
          // showSource is on (and the row is a structured message carrying a
          // provider) — otherwise render unchanged.
          const sourceProvider =
            showSource && typeof message !== 'string'
              ? ((message as BackendChatMessage).provider as ProviderId | undefined)
              : undefined;
          return (
            <MessageRow
              key={messageId || `msg-${index}`}
              messageId={messageId}
              userId={userId}
              isModFocus={!!modFocusId && messageId === modFocusId}
              intrinsicSizeCSS={intrinsicSizeCSS}
              sourceProvider={sourceProvider}
              sourceLogoSize={sourceLogoSize}
              message={message}
              isHighlighted={highlightedMessageId === messageId}
              moderationContext={moderationContext}
              emotes={emotes}
              isModerator={isModerator}
              broadcasterId={broadcasterId}
              onUsernameClick={onUsernameClick}
              onReplyClick={onReplyClick}
              onMessageCopy={onMessageCopy}
              onEmoteRightClick={onEmoteRightClick}
              onUsernameRightClick={onUsernameRightClick}
              onBadgeClick={onBadgeClick}
            />
          );
        })}
      </div>
    </div>
  );
});

export default ChatMessageList;
