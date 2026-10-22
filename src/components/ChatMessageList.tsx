import React, { useRef, useEffect, useLayoutEffect, useCallback, useMemo, useState, memo } from 'react';
import ChatMessage from './ChatMessage';
import HeldMessageRow from './chat/HeldMessageRow';
import { isHiddenEvent } from '../utils/chatEvents';
import { observeOffscreen } from '../utils/offscreenPause';
import { EmoteSet } from '../services/emoteService';
import { BackendChatMessage } from '../services/twitchChat';
import { ModerationContext } from '../hooks/useTwitchChat';
import { sameModerationContext } from './chat/moderationTagText';
import { useAppStore } from '../stores/AppStore';
import { useStreamerMode } from '../utils/streamerMode';
import { useChatUserStore } from '../stores/chatUserStore';
import type { ProviderId } from '../types/providers';
import { ScrollbarMarks } from './chat/ScrollbarMarks';
import { holdChatRow, moderateWithToast, ROW_HOLD_EVENT, useLineConfig, useModSliderSteps, useSliderConfig, type ModStep } from '../services/moderation';
import { lineButtons, type LineButton } from './chat/modLine';
import { bindLineStrips, bindLineWheel, isOnLineStrip, useLineArt, useLineButtons } from './chat/lineArt';
import { collectMarks, FALLBACK_MARK_KINDS } from './chat/scrollbarMarkLayout';
import { IS_MAC, IS_MOBILE } from '../utils/platform';

/**
 * How many of the newest rows always paint (see MessageRow). Enough to fill
 * a tall chat column at the smallest text size.
 */
const NEWEST_ROWS_PAINTED = 40;

/** On the phone the list meets the player band at its top edge: rows fade in
 *  over 24px there instead of being cut by a hard line. On the list's box, so
 *  the scroller inside keeps its own composited scrolling. */
const PHONE_TOP_FADE: React.CSSProperties = {
  WebkitMaskImage: 'linear-gradient(to bottom, transparent 0, #000 24px)',
  maskImage: 'linear-gradient(to bottom, transparent 0, #000 24px)',
};

/**
 * How long the pointer can be off a mod target (a line's buttons, a row with
 * the hover dock, the slider grip) before the hover hold lets go: long enough
 * to cross the gap between one line's buttons and the next, short enough that
 * leaving them feels immediate.
 */
const MOD_HOLD_GRACE_MS = 250;

/** No slider timeouts yet: one array, so the line's hooks see no change. */
const NO_STEPS: ModStep[] = [];

/** Where a mod action starts, besides the line buttons: a row that offers the
 *  hover dock or the slider (ChatMessage marks both), and the slider grip. */
const MOD_TARGET_SELECTOR = '[data-mod-dock], [data-mod-row], [data-mod-edge]';

function isOnModTarget(target: EventTarget | null): boolean {
  return target instanceof Element && (isOnLineStrip(target) || target.closest(MOD_TARGET_SELECTOR) !== null);
}

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
  /** One of the newest rows, which are on screen whenever the list follows. */
  isNewest: boolean;
  intrinsicSizeCSS: string;
  /** Blended feed: the platform logo ChatMessage draws in the name line. */
  sourceProvider: ProviderId | undefined;
  message: string | BackendChatMessage;
  isHighlighted: boolean;
  moderationContext: ModerationContext | null;
  emotes: EmoteSet | null;
  isModerator?: boolean;
  broadcasterId?: string;
  /** The platform being watched, so a row from one of the streamer's OTHER
   *  platforms can be told apart from this channel's own. */
  homeProvider?: ProviderId;
  /** The line buttons, one shared array for every row (null when off). */
  modLine: LineButton[] | null;
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
  if (prev.isNewest !== next.isNewest) return false;
  if (prev.intrinsicSizeCSS !== next.intrinsicSizeCSS) return false;
  if (prev.sourceProvider !== next.sourceProvider) return false;
  if (prev.message !== next.message) {
    if (typeof prev.message === 'string' || typeof next.message === 'string') return false;
    if (!prev.message.id || !next.message.id) return false;
    if (prev.message.id !== next.message.id) return false;
    // Same row re-stamped by Rust with new segments (see chatMessageAreEqual).
    if (prev.message.segments !== next.message.segments) return false;
    // A held AutoMod row folds in place once decided.
    if (prev.message.automodHeld !== next.message.automodHeld) return false;
  }
  if (prev.isHighlighted !== next.isHighlighted) return false;
  if (!sameModerationContext(prev.moderationContext, next.moderationContext)) return false;
  if (prev.emotes !== next.emotes) return false;
  if (prev.isModerator !== next.isModerator) return false;
  if (prev.broadcasterId !== next.broadcasterId) return false;
  if (prev.homeProvider !== next.homeProvider) return false;
  if (prev.modLine !== next.modLine) return false;
  return true;
};

const MessageRow = memo(function MessageRow({
  messageId,
  userId,
  isModFocus,
  isNewest,
  intrinsicSizeCSS,
  sourceProvider,
  message,
  isHighlighted,
  moderationContext,
  emotes,
  isModerator,
  broadcasterId,
  homeProvider,
  modLine,
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
    return !u?.themeHiddenInChat && !!(u?.atmosphereId || u?.cologne);
  });
  // Native virtualization for normal rows. Atmosphere rows paint always to
  // dodge the content-visibility compositing-ghost bug (see above). The newest
  // rows paint always too: a row lands below the fold, and under `auto` it is
  // laid out at the size hint until the browser marks it relevant, so a
  // wrapped row arrived at the hint and grew a frame later, jolting the list
  // a second time while the row slid in. They are on screen whenever the list
  // follows, so skipping them never saved anything.
  const paintAlways = hasAtmosphere || isNewest;
  const style = useMemo(
    () => ({
      contentVisibility: paintAlways ? ('visible' as const) : ('auto' as const),
      // Off-screen size hint, computed per-user from font size, spacing, and
      // whether timestamps are on. Ignored when content-visibility is visible.
      containIntrinsicBlockSize: paintAlways ? undefined : intrinsicSizeCSS,
    }),
    [paintAlways, intrinsicSizeCSS],
  );
  // An Atmosphere or Cologne row paints always (above), so its infinite wash
  // would keep animating while scrolled out of the list. The observer marks it
  // off-screen and globals.css pauses the wash until it scrolls back.
  const rowRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = rowRef.current;
    if (!hasAtmosphere || !el) return;
    return observeOffscreen(el);
  }, [hasAtmosphere]);
  return (
    <div
      ref={rowRef}
      data-message-id={messageId || undefined}
      className={`chat-message-row${isModFocus ? ' is-mod-focus' : ''}${
        typeof message !== 'string' && message.metadata?.from_backfill ? ' is-backfill' : ''
      }`}
      style={style}
    >
      {typeof message !== 'string' && message.automodHeld ? (
        <HeldMessageRow row={message.automodHeld} />
      ) : (
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
        homeProvider={homeProvider}
        sourceMark={sourceProvider}
        modLine={modLine}
      />
      )}
    </div>
  );
}, messageRowAreEqual);

interface ChatMessageListProps {
  messages: (string | BackendChatMessage)[];
  /** A quiet line in the middle while there are no rows ("No recent messages"
   *  once the join's history answered with none). Gone with the first row. */
  emptyNote?: string | null;
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
  /** The platform being watched, so a row from one of the streamer's OTHER
   *  platforms can be told apart from this channel's own. */
  homeProvider?: ProviderId;
  /** Blended view: mark each row with its source platform (a provider-colored
   *  left stripe), so a merged multi-source feed shows where each message is from.
   *  Off everywhere else, so normal single-source chat is untouched. */
  showSource?: boolean;
  /** Something above this list already cleared the floating header, so it must
   *  not reserve that space a second time. */
  headerSpaceReserved?: boolean;
  /** How tall the floating header over this list is, measured by the host (a
   *  hype train makes it much taller). Defaults to the plain header's 40px. */
  headerInset?: number;
  /** Ids of this channel's pinned messages, for the scrollbar's pinned marks. */
  pinnedIds?: ReadonlySet<string>;
  /** Filled with this list's eased scroll to the live bottom. Call it in the
   *  same handler that un-pauses, before the re-render pins the list. */
  scrollToBottomRef?: React.RefObject<(() => void) | null>;
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
  messages: liveMessages,
  emptyNote,
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
  homeProvider,
  showSource,
  headerSpaceReserved,
  headerInset = 40,
  pinnedIds,
  scrollToBottomRef,
}: ChatMessageListProps) {
  // While a moderator points where a mod action starts (the hover effect
  // below), or presses a row to slide or drag it, the list keeps drawing the
  // rows that were on screen, so nothing moves and nothing is trimmed away
  // however fast chat runs; it catches up to live when the pointer leaves and
  // the press ends. The frozen array is the store's own copy-on-write
  // snapshot, so holding it costs one reference. Everything below draws
  // `messages`.
  const [frozenView, setFrozenView] = useState<ChatMessageListProps['messages'] | null>(null);
  const messages = frozenView ?? liveMessages;
  const shownRef = useRef(messages);
  useLayoutEffect(() => {
    shownRef.current = messages;
  });
  const viewFrozenRef = useRef(false);
  // Why the view is held: the pointer rests on a mod target, a press holds a
  // row, or both. It thaws only when neither is left.
  const freezeRef = useRef({ hover: false, press: false });
  // A wheel scroll lets go of the hover hold, which then waits for the
  // pointer to move again, so scrolling never fights it.
  const hoverMutedRef = useRef(false);
  const syncFreeze = useCallback(() => {
    const want = freezeRef.current.hover || freezeRef.current.press;
    if (want === viewFrozenRef.current) return;
    viewFrozenRef.current = want;
    setFrozenView(want ? shownRef.current : null);
  }, []);
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
  // The row a moderator is pressing to slide or drag (ModSlider, the drag
  // pickup), and where it sat on screen. A press also freezes the view, so the
  // 100-row buffer cannot trim the held row away mid-slide (which cancels the
  // slide). While it is held the list stops following new messages and, after
  // every update, scrolls by however far the row moved, so the row stays under
  // the pointer. The browser's own scroll anchoring was not enough: it let go
  // after a few hundred milliseconds of trimming. The hold ends on its own
  // when the button comes up or the row leaves the list, so it can never
  // stick; then the list catches up to the live bottom unless it was paused.
  const rowHoldRef = useRef<{ row: HTMLElement; top: number } | null>(null);
  const isPausedRef = useRef(isPaused);
  useEffect(() => {
    isPausedRef.current = isPaused;
  }, [isPaused]);
  const isRowHeld = () => {
    const held = rowHoldRef.current;
    if (held && !held.row.isConnected) rowHoldRef.current = null;
    return rowHoldRef.current !== null;
  };
  /** Scroll by however far the held row moved, so it stays put on screen. */
  const keepHeldRowStill = () => {
    const held = rowHoldRef.current;
    const el = containerRef.current;
    if (!held || !el) return;
    const drift = held.row.getBoundingClientRect().top - held.top;
    if (Math.abs(drift) < 0.5) return;
    isScrollingProgrammatically.current = true;
    el.scrollTop += drift;
    setTimeout(() => {
      isScrollingProgrammatically.current = false;
    }, 60);
  };
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const freeze = freezeRef.current;
    const release = () => {
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
      freeze.press = false;
      syncFreeze();
      if (!rowHoldRef.current) return;
      rowHoldRef.current = null;
      if (isPausedRef.current || userScrolledUpRef.current) return;
      isScrollingProgrammatically.current = true;
      el.scrollTop = el.scrollHeight;
      wasAtBottomRef.current = true;
      setTimeout(() => {
        isScrollingProgrammatically.current = false;
      }, 150);
    };
    const onHold = (e: Event) => {
      if ((e as CustomEvent<{ on?: boolean }>).detail?.on !== true) {
        release();
        return;
      }
      const row = e.target as HTMLElement;
      rowHoldRef.current = { row, top: row.getBoundingClientRect().top };
      freeze.press = true;
      syncFreeze();
      window.addEventListener('pointerup', release, true);
      window.addEventListener('pointercancel', release, true);
    };
    el.addEventListener(ROW_HOLD_EVENT, onHold);
    return () => {
      el.removeEventListener(ROW_HOLD_EVENT, onHold);
      window.removeEventListener('pointerup', release, true);
      window.removeEventListener('pointercancel', release, true);
      freeze.press = false;
      syncFreeze();
    };
  }, [syncFreeze]);

  // Per-row CSS `contain-intrinsic-block-size` placeholder size. The browser
  // uses this for off-screen messages (content-visibility: auto). Picking a
  // value close to the real rendered height prevents a layout shift on the
  // FIRST render of each message — the prior hardcoded 50px was tuned for
  // single-line messages, which was way off when timestamps add a second line.
  // The `auto` keyword still lets the browser remember the actual size after
  // first render, so this is just the initial-paint guess.
  const chatDesign = useAppStore((s) => s.settings.chat_design);
  const hiddenEvents = useAppStore((s) => s.settings.chat_events?.hidden_provider_events);
  const intrinsicSizeCSS = useMemo(() => {
    const fontSize = chatDesign?.font_size ?? 14;
    const messageSpacing = chatDesign?.message_spacing ?? 8;
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

  // Line buttons. Rust's steps for the viewer's line list are resolved here,
  // once for the whole list, and every row gets the same array: no per-row
  // hooks or subscriptions on the hot path. Nothing is asked for unless the
  // viewer moderates this channel and has them on.
  const lineOn = !!isModerator && chatDesign?.mod_line_buttons === true;
  const lineConfig = useLineConfig();
  const lineSteps = useModSliderSteps(homeProvider ?? 'twitch', lineConfig, lineOn);
  // What the rows get: only that the buttons are on, so a scrolled length
  // (below) never re-renders a row.
  const modLine = useMemo(
    () => (lineOn && lineSteps && lineSteps.length > 0 ? lineButtons(lineSteps) : null),
    [lineOn, lineSteps],
  );
  const lineActive = modLine !== null;
  // Hover: only the line under the pointer shows them, pushing its text over.
  const lineShow = chatDesign?.mod_line_show === 'always' ? 'always' : 'hover';
  // A scroll over the buttons walks a timeout up or down the slider's
  // timeouts (its Steps, as Rust applies them on this platform).
  const sliderConfig = useSliderConfig();
  const ladderSteps = useModSliderSteps(homeProvider ?? 'twitch', sliderConfig, lineOn);
  const lineLadder = useMemo(() => ladderSteps?.filter((s) => s.action === 'timeout') ?? NO_STEPS, [ladderSteps]);
  const { buttons: lineNow, nudge: nudgeLine, reset: resetLine, around: aroundLine } = useLineButtons(lineOn ? lineSteps : null, lineLadder);
  // Timeouts drawn as wide as any length a scroll can reach.
  const lineFit = useMemo(
    () => [...lineLadder, ...(lineSteps ?? []).filter((s) => s.action === 'timeout')].map((s) => s.label),
    [lineLadder, lineSteps],
  );
  // The buttons' picture, drawn once for the list; every strip shows it.
  const lineArt = useLineArt(lineNow, chatDesign?.font_size ?? 14, lineFit, modLine);
  const lineArtRef = useRef<{ art: NonNullable<typeof lineArt>; buttons: LineButton[] } | null>(null);
  useLayoutEffect(() => {
    lineArtRef.current = lineArt && lineNow ? { art: lineArt, buttons: lineNow } : null;
  });
  const lineWheelRef = useRef({ nudge: nudgeLine, reset: resetLine, around: aroundLine });
  useLayoutEffect(() => {
    lineWheelRef.current = { nudge: nudgeLine, reset: resetLine, around: aroundLine };
  });
  // Pointing where a mod action starts freezes the view (mouse and pen; a
  // touch has no hover), so a fast chat cannot pull the target away while you
  // aim, however fast it runs: a line's buttons, a row that offers the hover
  // dock (from the row itself: by the time the pointer reaches the dock above
  // it, a fast chat has already moved the row), the slider grip or tile
  // handle. Moving between lines passes through the gap below each strip, so
  // the view thaws only after the pointer has been off every target for
  // MOD_HOLD_GRACE_MS, or at once on a wheel scroll; then the list catches up
  // to live.
  useEffect(() => {
    const el = containerRef.current;
    if (!el || !isModerator) return;
    let graceTimer: number | undefined;
    const thaw = () => {
      window.clearTimeout(graceTimer);
      freezeRef.current.hover = false;
      syncFreeze();
    };
    const onOver = (e: PointerEvent) => {
      if (e.pointerType === 'touch' || hoverMutedRef.current || !isOnModTarget(e.target)) return;
      window.clearTimeout(graceTimer);
      freezeRef.current.hover = true;
      syncFreeze();
    };
    const onOut = (e: PointerEvent) => {
      if (!freezeRef.current.hover || !isOnModTarget(e.target) || isOnModTarget(e.relatedTarget)) return;
      window.clearTimeout(graceTimer);
      graceTimer = window.setTimeout(thaw, MOD_HOLD_GRACE_MS);
    };
    // After a wheel scroll, a real move of the pointer aims again.
    const onMove = (e: PointerEvent) => {
      if (!hoverMutedRef.current || (e.movementX === 0 && e.movementY === 0)) return;
      hoverMutedRef.current = false;
      onOver(e);
    };
    const onLeave = () => {
      hoverMutedRef.current = false;
    };
    el.addEventListener('pointerover', onOver);
    el.addEventListener('pointerout', onOut);
    el.addEventListener('pointermove', onMove);
    el.addEventListener('pointerleave', onLeave);
    return () => {
      el.removeEventListener('pointerover', onOver);
      el.removeEventListener('pointerout', onOut);
      el.removeEventListener('pointermove', onMove);
      el.removeEventListener('pointerleave', onLeave);
      thaw();
    };
  }, [isModerator, syncFreeze]);
  // One set of listeners on the list serves every row's line buttons.
  // - The button under the pointer lights up (`bindLineStrips`).
  // - A press without the hover hold (touch) holds the row until it lifts.
  // - A press keeps the composer's focus and starts no text selection.
  // - A click picks the button under the pointer and sends it through
  //   `moderate_chatter`, with Undo in the toast for a timeout or ban.
  useEffect(() => {
    const el = containerRef.current;
    if (!el || !lineActive) return;
    const onDown = (e: PointerEvent) => {
      if (!isOnLineStrip(e.target) || e.button !== 0 || viewFrozenRef.current) return;
      holdChatRow(e.target as HTMLElement, true);
    };
    el.addEventListener('pointerdown', onDown);
    const unbind = bindLineStrips(el, () => lineArtRef.current, (hit) => void moderateWithToast(hit.request));
    // The wheel over a line's buttons walks its timeout's length; it never
    // reaches the list's own wheel handling, so chat holds still meanwhile.
    const unbindWheel = bindLineWheel(
      el,
      () => lineArtRef.current,
      (index, dir) => lineWheelRef.current.nudge(index, dir),
      () => lineWheelRef.current.reset(),
      (index) => lineWheelRef.current.around(index),
    );
    return () => {
      el.removeEventListener('pointerdown', onDown);
      unbind();
      unbindWheel();
    };
  }, [lineActive]);


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
    // Scrolling is not aiming: a wheel over a mod target lets go of the hover
    // hold at once, so the wheel scrolls the live feed (up pauses it as
    // usual), and the hold waits for the pointer to move again. A press still
    // holds its row.
    hoverMutedRef.current = true;
    if (freezeRef.current.hover) {
      freezeRef.current.hover = false;
      syncFreeze();
    }
  }, [onPauseIntent, syncFreeze]);

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
      // A row is being slid; it must stay under the pointer.
      if (isRowHeld()) {
        keepHeldRowStill();
        return;
      }
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

  // A layout effect, so the pin lands before the new row is painted: pinned
  // after paint, the row first drew below the fold and the list jumped to it a
  // frame later, mid-entrance.
  useLayoutEffect(() => {
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
    // A row being slid stays put; the hold's release catches up.
    if (!isChannelLoad && isRowHeld()) {
      keepHeldRowStill();
      return;
    }

    // NOT PAUSED: Always scroll to bottom, now (before paint), then once more
    // after content-visibility resolves for anything further up.
    isScrollingProgrammatically.current = true;
    containerRef.current.scrollTop = containerRef.current.scrollHeight;
    wasAtBottomRef.current = true;

    // Second scroll: short delay to catch content-visibility final height calculation
    // This fixes the issue where new messages appear partially behind the input box
    setTimeout(() => {
      if (!containerRef.current || isPaused) return;
      if (!isChannelLoad && userScrolledUpRef.current) return;
      if (isResumeAnimatingRef.current) return;
      if (isRowHeld()) return;
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

  // Hand this list's own glide to its parent. One window-wide slot used to
  // serve every list, so in a window with several (MultiChat tabs and splits,
  // docked chats) Resume glided whichever list mounted last while the visible
  // one snapped to the bottom.
  useEffect(() => {
    if (!scrollToBottomRef) return;
    scrollToBottomRef.current = scrollToBottom;
    return () => {
      if (scrollToBottomRef.current === scrollToBottom) scrollToBottomRef.current = null;
    };
  }, [scrollToBottom, scrollToBottomRef]);

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

  // Scrollbar marks: Rust stamps which rows are marked; this keeps the ones the
  // viewer chose to see. One pass per feed change, no row re-renders. A phone
  // draws no scrollbar, so there is nothing for the ticks to sit on.
  const marksOn = !IS_MOBILE && (chatDesign?.scrollbar_marks ?? true);
  const markKinds = chatDesign?.scrollbar_mark_kinds;
  const shownKinds = useMemo(
    () => new Set<string>(marksOn ? (markKinds ?? FALLBACK_MARK_KINDS) : []),
    [marksOn, markKinds],
  );
  const marks = useMemo(
    () => collectMarks(messages, getMessageId, shownKinds, pinnedIds),
    [messages, getMessageId, shownKinds, pinnedIds],
  );

  return (
    <div className="relative h-full" style={IS_MOBILE ? PHONE_TOP_FADE : undefined}>
      <div
        ref={containerRef}
        // On the phone the bar shows only while the list is paused (sn-chat-live
        // in styles/mobile.css): pinned to the newest line it moves every message.
        className={`h-full overflow-y-auto overflow-x-hidden scrollbar-thin${isPaused ? '' : ' sn-chat-live'}`}
        data-line-show={lineActive ? lineShow : undefined}
        onScroll={handleScroll}
        onWheel={handleWheel}
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        style={{
          overflowAnchor: 'auto',
          '--sn-emote-scale': emoteScale,
          // Chat GIF size (ChatMessage); Rust holds the saved value to 0.5-2.
          '--sn-gif-scale': chatDesign?.gif_scale ?? 1,
          '--sn-emote-margin': `${emoteMargin}rem`,
          '--sn-backfill-opacity': Math.max(0.3, Math.min(1, (chatDesign?.backfill_opacity ?? 100) / 100)),
          '--sn-badge-scale': Math.max(0.5, Math.min(2.5, chatDesign?.badge_scale ?? 1)),
          ...lineArt?.vars,
        } as React.CSSProperties}
      >
        {/* Messages container with native virtualization. The top padding clears
            the chat header, which floats OVER this column rather than sitting
            above it, so the space has to be reserved here — inside the scroll
            content — or the oldest message hides behind it at the top of
            scrollback. When something above already reserved it (the
            combined-chat bar sits below the header and pushes this list down),
            reserving it twice leaves a dead gap instead.
            data-entrance drives a CSS-only arrival animation on live rows
            (history rows carry .is-backfill and are excluded by the selector). */}
        <div
          ref={contentRef}
          className={`flex flex-col min-h-full justify-end${chatDesign?.alternating_backgrounds ? ' chat-striped' : ''}`}
          style={headerSpaceReserved ? undefined : { paddingTop: headerInset }}
          // How much of the list's top the floating header covers; the mod
          // slider's chip goes under a line that close to it.
          data-header-inset={headerSpaceReserved ? 0 : headerInset}
          data-entrance={chatDesign?.message_entrance && chatDesign.message_entrance !== 'none' ? chatDesign.message_entrance : undefined}
        >
          {messages.map((message, index) => {
            const messageId = getMessageId(message);
            // Event categories the viewer turned off for this platform.
            if (typeof message !== 'string' && isHiddenEvent(message, hiddenEvents)) return null;

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
              : message.match(/(?:^@|;)user-id=([^;]+)/)?.[1];

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

            // Blended feed: each message leads its name line with its source
            // platform's logo so a merged multi-source feed is readable at a
            // glance. Only when showSource is on (and the row is a structured
            // message carrying a provider); otherwise render unchanged.
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
                isNewest={index >= messages.length - NEWEST_ROWS_PAINTED}
                intrinsicSizeCSS={intrinsicSizeCSS}
                sourceProvider={sourceProvider}
                message={message}
                isHighlighted={highlightedMessageId === messageId}
                moderationContext={moderationContext}
                emotes={emotes}
                isModerator={isModerator}
                broadcasterId={broadcasterId}
                homeProvider={homeProvider}
                modLine={modLine}
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
      {marksOn && (
        <ScrollbarMarks scrollerRef={containerRef} contentRef={contentRef} marks={marks} overlay={IS_MAC} />
      )}
      {emptyNote && liveMessages.length === 0 && (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center px-6">
          <p className="text-center text-[13px] text-textMuted">{emptyNote}</p>
        </div>
      )}
    </div>
  );
});

export default ChatMessageList;
