// The pinned mentions at the top of a chat (SN-284 v2). Rust decides which
// mentions are pinned, where each lives, how long it counts down and what its
// row looks like; a surface draws the stack it is given, oldest at the top and
// the newest at the bottom, each pin the real row of the chat it was said in
// (the shell's own `ChatMessage` in its pinned variant, outside the chat list).
//
// - A pin of another chat carries a channel mark above its row.
// - A press on the row opens it (`onOpenOwn` scrolls to it in this chat,
//   `onOpenOther` opens the Mentions inbox at it); X closes one; Clear all sits
//   on the count line and closes every pin the count names, by the ids that
//   were drawn when the press began.
// - While the pointer moved here in the last 2 seconds, or a finger is down,
//   nothing is removed or moved (`pinsHold.ts`).
// - The pins in view are reported (`reportVisible`) so Rust counts only what
//   the viewer can see.
// - The forms come from Rust's stack layout: `full` (every row, scrolling
//   inside, newest in view), `floor` (the newest row and the count),
//   `count_line` (one line with the count inline).

import { memo, useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { X } from 'phosphor-react';
import ChatMessage from '../ChatMessage';
import { ProviderLogo } from '../ProviderLogo';
import { Tooltip } from '../ui/Tooltip';
import { useChatUserStore } from '../../stores/chatUserStore';
import { NameColorToneContext } from '../../hooks/useNameColor';
import { releaseRowImagesOnUnmount } from '../../utils/releaseImages';
import { observeOffscreen } from '../../utils/offscreenPause';
import { cosmeticsKey } from '../../utils/chatterIdentity';
import { OVER_VIDEO_CLASS, useOverVideoStyle, useThemeDark } from '../../mobile/chat/overVideo';
import { Logger } from '../../utils/logger';
import { HOLD_MS, dayLabelFor, mergeHeld } from './pinsHold';
import type { MentionPin } from '../../types';
import type { ProviderId } from '../../types/providers';

export type PinsForm = 'full' | 'floor' | 'strip' | 'count_line';

const dismissIds = (ids: string[], undo: () => void) => {
  invoke('dismiss_mention_pins', { ids }).catch((err) => {
    Logger.warn('[MentionPins] dismiss failed:', err);
    undo();
  });
};

/** Warms the chatter store for a pin's author the way a chat row's list does,
 *  with the pin's own channel (never the surface's): the paint, 7TV badge and
 *  third-party badges of someone who has not chatted in this window load. */
export function useWarmPinChatter(pin: MentionPin, warm: boolean): void {
  const row = pin.row;
  const key = row?.user_id ? cosmeticsKey(row.user_id, (row.provider ?? 'twitch') as ProviderId) : null;
  const channelId = row?.tags?.['room-id'] ?? '';
  const channelName = pin.entry.channel_login || pin.channel_mark?.channel || '';
  useEffect(() => {
    if (!warm || !key || !row) return;
    if (useChatUserStore.getState().users.has(key)) return;
    useChatUserStore.getState().addUser(
      {
        userId: key,
        username: row.username || pin.entry.sender_login,
        displayName: row.display_name || pin.entry.sender_name,
        color: row.color || '',
      },
      { channelId, channelName },
    );
  }, [warm, key, row, channelId, channelName, pin.entry.sender_login, pin.entry.sender_name]);
}

/** A small dot for the clock: counting, or held while the viewer is away. */
function ClockDot({ clock }: { clock: MentionPin['clock'] }) {
  if (clock === 'none') return null;
  const dot = (
    <span
      data-clock={clock}
      className={`inline-block h-[5px] w-[5px] rounded-full ${
        clock === 'held' ? 'border border-textMuted/70' : 'bg-accent/70'
      }`}
    />
  );
  if (clock === 'running') return <span aria-hidden>{dot}</span>;
  return (
    <Tooltip content="Paused. A pin's time counts while you're using StreamNook." side="top">
      <span aria-label="Paused">{dot}</span>
    </Tooltip>
  );
}

interface PinRowProps {
  pin: MentionPin;
  phone: boolean;
  /** Draw the chatter store warm-up (never for the settings preview). */
  warm: boolean;
  onOpen: (pin: MentionPin) => void;
  onDismiss: (pin: MentionPin) => void;
  onUsernameClick?: (pin: MentionPin, e: React.MouseEvent) => void;
}

/** One pin: its channel mark, its real row, its clock and X. */
const PinRow = memo(function PinRow({ pin, phone, warm, onOpen, onDismiss, onUsernameClick }: PinRowProps) {
  useWarmPinChatter(pin, warm);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const stop = observeOffscreen(el);
    const release = releaseRowImagesOnUnmount(el);
    return () => {
      stop();
      release?.();
    };
  }, []);
  const mark = pin.channel_mark;
  const info = useMemo(
    () => ({ rev: pin.rev, dayLabel: pin.day ? dayLabelFor(pin.entry.at_ms) : null }),
    [pin.rev, pin.day, pin.entry.at_ms],
  );
  const reply = pin.entry.kind === 'reply';
  return (
    <div
      ref={ref}
      data-pin-id={pin.entry.id}
      data-pin-kind={pin.entry.kind}
      dir="ltr"
      className="relative group"
    >
      {mark && (
        <div className="flex items-center gap-1 px-3 pt-1 text-[11px] leading-4 text-textMuted min-w-0">
          <ProviderLogo provider={mark.platform as ProviderId} size={11} className="flex-shrink-0" />
          <span className="truncate">{mark.channel}</span>
        </div>
      )}
      {/* The row opens the pin. Links, emotes and the name keep their own
          clicks (the name opens the card of the pin's own chat). */}
      <div
        role="button"
        tabIndex={0}
        aria-label={`${pin.entry.sender_name || pin.entry.sender_login} ${reply ? 'replied to you' : 'mentioned you'}. Open it`}
        className="cursor-pointer pr-9"
        onClick={(e) => {
          if ((e.target as HTMLElement).closest('a, button, input')) return;
          onOpen(pin);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            onOpen(pin);
          }
        }}
      >
        <ChatMessage
          message={pin.row}
          pinned={info}
          homeProvider={(pin.row.provider ?? 'twitch') as ProviderId}
          onUsernameClick={
            onUsernameClick
              ? (_id, _login, _name, _color, _badges, e) => onUsernameClick(pin, e)
              : undefined
          }
        />
      </div>
      <div className="absolute right-1.5 top-1.5 flex items-center gap-1.5">
        {pin.run > 1 && <span className="text-[11px] font-semibold tabular-nums text-textMuted">×{pin.run}</span>}
        <ClockDot clock={pin.clock} />
        <button
          type="button"
          // A tap here never takes the keyboard down mid-reply.
          onPointerDown={(e) => e.preventDefault()}
          onClick={() => onDismiss(pin)}
          aria-label="Dismiss this pinned mention"
          className={`flex items-center justify-center rounded-lg text-textSecondary/70 transition-colors hover:bg-white/10 hover:text-textPrimary ${
            phone ? 'h-8 w-8' : 'h-6 w-6'
          }`}
        >
          <X size={13} weight="bold" />
        </button>
      </div>
    </div>
  );
});

export interface MentionPinsBlockProps {
  /** Rust's answer for this surface, oldest first. */
  pins: readonly MentionPin[];
  /** How the stack draws this card. */
  form: PinsForm;
  /** The slot's height from the stack; the block never grows past it. */
  height?: number;
  /** Cards the stack folded into this card's count line ("N more cards"). */
  moreCards?: number;
  /** This surface is on screen (`chatSurfaceShown`): only then are pins reported in view. */
  shown: boolean;
  phone?: boolean;
  /** Chat drawn over the video: dark ink and backing in every theme. */
  overVideo?: boolean;
  /** The pin's chat is this surface's chat: scroll to it. */
  onOpenOwn: (pin: MentionPin) => void;
  /** Another chat's pin: open the Mentions inbox at it. */
  onOpenOther: (pin: MentionPin) => void;
  /** The pins in view changed (every id; empty when not shown). */
  reportVisible?: (ids: string[]) => void;
  /** A press on a pin's name, with the pin's own chat. */
  onUsernameClick?: (pin: MentionPin, e: React.MouseEvent) => void;
  /** Warm the chatter store (false for the settings preview). */
  warm?: boolean;
  /** Reports the block's drawn heights per form to the stack's cache. */
  onMeasure?: (m: { form: PinsForm; height: number }) => void;
  className?: string;
}

/** The pins block: a stack of real rows with a count line. */
export function MentionPinsBlock({
  pins: live,
  form,
  height,
  moreCards = 0,
  shown,
  phone = false,
  overVideo = false,
  onOpenOwn,
  onOpenOther,
  reportVisible,
  onUsernameClick,
  warm = true,
  onMeasure,
  className = '',
}: MentionPinsBlockProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // ---- hold ---------------------------------------------------------------
  const [frozen, setFrozen] = useState<MentionPin[] | null>(null);
  const liveRef = useRef(live);
  liveRef.current = live;
  const holdTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const fingerDown = useRef(false);
  const release = useCallback(() => {
    clearTimeout(holdTimer.current);
    fingerDown.current = false;
    setFrozen(null);
  }, []);
  const hold = useCallback(() => {
    setFrozen((f) => f ?? [...liveRef.current]);
    clearTimeout(holdTimer.current);
    if (!fingerDown.current) holdTimer.current = setTimeout(release, HOLD_MS);
  }, [release]);
  useEffect(() => {
    if (!frozen) return;
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') release();
    };
    const onTouchEnd = () => {
      fingerDown.current = false;
      clearTimeout(holdTimer.current);
      holdTimer.current = setTimeout(release, HOLD_MS);
    };
    window.addEventListener('blur', release);
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('touchcancel', release);
    window.addEventListener('touchend', onTouchEnd);
    return () => {
      window.removeEventListener('blur', release);
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('touchcancel', release);
      window.removeEventListener('touchend', onTouchEnd);
    };
  }, [frozen, release]);
  useEffect(() => () => clearTimeout(holdTimer.current), []);

  // ---- X ------------------------------------------------------------------
  const [gone, setGone] = useState<ReadonlySet<string>>(() => new Set());
  const shownPins = useMemo(() => {
    const base = frozen ? mergeHeld(frozen, live) : live;
    return base.filter((p) => !gone.has(p.entry.id));
  }, [frozen, live, gone]);
  const putAway = useCallback((ids: string[]) => {
    setGone((prev) => new Set([...prev, ...ids]));
    dismissIds(ids, () =>
      setGone((prev) => {
        const next = new Set(prev);
        ids.forEach((i) => next.delete(i));
        return next;
      }),
    );
  }, []);
  // Ids Rust no longer pins leave the set as it grows.
  useEffect(() => {
    setGone((prev) => {
      if (prev.size === 0) return prev;
      const ids = new Set(live.map((p) => p.entry.id));
      const next = new Set([...prev].filter((i) => ids.has(i)));
      return next.size === prev.size ? prev : next;
    });
  }, [live]);

  // Clear all takes the ids drawn when the press began, so a pin that lands
  // between the press and the click is never closed unseen.
  const clearIds = useRef<string[]>([]);
  // A press that began on the button; the keyboard (no press) closes what is drawn now.
  // Either way the press is spent: the next one starts from nothing.
  const clearAll = () => {
    const ids = clearIds.current.length ? clearIds.current : shownPins.map((p) => p.entry.id);
    clearIds.current = [];
    putAway(ids);
  };

  // ---- what is in view ----------------------------------------------------
  const drawn = form === 'full' ? shownPins : form === 'count_line' ? [] : shownPins.slice(-1);
  const drawnKey = drawn.map((p) => p.entry.id).join('|');
  useEffect(() => {
    if (!reportVisible) return;
    if (!shown || drawn.length === 0 || typeof IntersectionObserver === 'undefined') {
      reportVisible([]);
      return;
    }
    const root = rootRef.current;
    if (!root) return;
    const visible = new Set<string>();
    let last = '';
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const id = (e.target as HTMLElement).dataset.pinId;
          if (!id) continue;
          if (e.isIntersecting) visible.add(id);
          else visible.delete(id);
        }
        const key = [...visible].sort().join('|');
        if (key === last) return;
        last = key;
        reportVisible([...visible]);
      },
      // The viewport as root: every clip above (the stack, a scroll, the chat
      // column) counts. Half the row must show.
      { root: null, threshold: 0.5 },
    );
    root.querySelectorAll<HTMLElement>('[data-pin-id]').forEach((el) => io.observe(el));
    return () => io.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- drawnKey stands for `drawn`
  }, [shown, drawnKey, reportVisible]);
  // Not mounted any more: nothing of it is in view.
  useEffect(() => () => reportVisible?.([]), [reportVisible]);

  const shownPinsLength = shownPins.length;
  // ---- the newest stays in view -------------------------------------------
  const lastId = shownPins[shownPins.length - 1]?.entry.id;
  const stuckToEnd = useRef(true);
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || form !== 'full' || !stuckToEnd.current || frozen) return;
    el.scrollTop = el.scrollHeight;
  }, [lastId, form, frozen, shownPins.length]);

  // ---- measuring (the stack keeps the last height of each form) -----------
  // The list pans only when its rows overflow, and only from a touch that starts
  // on a row; otherwise a touch on it is just a tap and never a scroll.
  useEffect(() => {
    const inner = scrollRef.current;
    if (!inner || typeof ResizeObserver === 'undefined') return;
    const sync = () => {
      inner.style.touchAction = inner.scrollHeight > inner.clientHeight + 1 ? 'pan-y' : 'none';
    };
    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(inner);
    return () => ro.disconnect();
  }, [form, shownPinsLength]);

  useEffect(() => {
    const el = rootRef.current;
    if (!el || !onMeasure || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      const inner = scrollRef.current;
      // The full form reports its natural height (every row), not the cap.
      const h = form === 'full' && inner ? inner.scrollHeight : el.getBoundingClientRect().height;
      if (h > 0) onMeasure({ form, height: Math.round(h) });
    });
    ro.observe(el);
    if (scrollRef.current) ro.observe(scrollRef.current);
    return () => ro.disconnect();
  }, [form, onMeasure]);

  const overVideoVars = useOverVideoStyle(overVideo);
  const themeDark = useThemeDark();
  const outerTone = useContext(NameColorToneContext);
  const tone = overVideo && !themeDark ? true : outerTone;

  const open = useCallback(
    (pin: MentionPin) => {
      release();
      if (pin.channel_mark) onOpenOther(pin);
      else onOpenOwn(pin);
    },
    [onOpenOther, onOpenOwn, release],
  );

  if (shownPins.length === 0) return null;
  const total = shownPins.length;
  const countText = `${total} pinned ${total === 1 ? 'mention' : 'mentions'}${
    moreCards > 0 ? ` · ${moreCards} more ${moreCards === 1 ? 'card' : 'cards'}` : ''
  }`;

  const surface = phone
    ? 'chrome-glaze !rounded-2xl'
    : 'glass-flyout rounded-xl';
  const style: CSSProperties = { ...overVideoVars, maxHeight: height && height > 0 ? height : undefined };

  const countLine = (
    <div className="flex items-center justify-between gap-2 px-3 py-1 text-[11px] leading-4 text-textMuted" data-pins-count>
      <span className="truncate tabular-nums">{countText}</span>
      {total >= 2 && (
        <button
          type="button"
          onPointerDown={(e) => {
            e.preventDefault();
            clearIds.current = shownPins.map((p) => p.entry.id);
          }}
          onClick={clearAll}
          className="flex-shrink-0 rounded-md px-1.5 py-0.5 font-semibold text-textSecondary transition-colors hover:bg-white/10 hover:text-textPrimary"
        >
          Clear all
        </button>
      )}
    </div>
  );

  const row = (pin: MentionPin) => (
    <PinRow
      key={pin.entry.id}
      pin={pin}
      phone={phone}
      warm={warm}
      onOpen={open}
      onDismiss={(p) => putAway([p.entry.id])}
      onUsernameClick={onUsernameClick}
    />
  );

  return (
    <NameColorToneContext.Provider value={tone}>
      <div
        ref={rootRef}
        data-mention-pins={form}
        className={`pointer-events-auto relative flex w-full flex-col overflow-hidden ${surface} ${
          overVideo ? OVER_VIDEO_CLASS : ''
        } ${className}`}
        style={style}
        onPointerMove={(e) => {
          if (e.pointerType !== 'touch') hold();
        }}
        onPointerDown={(e) => {
          if (e.pointerType === 'touch') {
            fingerDown.current = true;
            hold();
          }
        }}
        onPointerLeave={(e) => {
          if (e.pointerType !== 'touch') {
            clearTimeout(holdTimer.current);
            holdTimer.current = setTimeout(release, HOLD_MS);
          }
        }}
      >
        {form === 'count_line' ? (
          <div className="flex items-center justify-between gap-2 px-3 py-1 text-[12px] text-textSecondary" data-pins-count>
            <span className="truncate tabular-nums">{countText}</span>
            {total >= 2 && (
              <button
                type="button"
                onPointerDown={(e) => {
                  e.preventDefault();
                  clearIds.current = shownPins.map((p) => p.entry.id);
                }}
                onClick={clearAll}
                className="flex-shrink-0 rounded-md px-1.5 py-0.5 font-semibold transition-colors hover:bg-white/10 hover:text-textPrimary"
              >
                Clear all
              </button>
            )}
          </div>
        ) : form === 'full' ? (
          <>
            <div
              ref={scrollRef}
              className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
              onScroll={(e) => {
                const el = e.currentTarget;
                stuckToEnd.current = el.scrollHeight - el.scrollTop - el.clientHeight < 8;
              }}
            >
              {shownPins.map(row)}
            </div>
            {countLine}
          </>
        ) : (
          <>
            {row(shownPins[shownPins.length - 1])}
            {countLine}
          </>
        )}
      </div>
    </NameColorToneContext.Provider>
  );
}

export default MentionPinsBlock;
