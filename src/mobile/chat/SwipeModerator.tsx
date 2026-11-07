// A sideways swipe on a chat row, the same on Android as on the iPhone:
//
// - Swipe RIGHT to moderate (moderators): the desktop mod slider with a finger.
//   Rust lays the slider out for each press, from where the thumb landed, and
//   says every word with the chatter's name in it (`get_touch_mod_slider`,
//   services/touch_mod_slider.rs); this follows the finger, draws the chosen
//   style over the row, buzzes on each step and on release sends the armed step
//   through the one mod door. Short of the cancel zone, or with the finger
//   strayed `leash` px above or below the message, the chip says "Let go to
//   cancel" and a release does nothing. Ban sits at the end of the pull and arms
//   only after the finger rests there; a release before it arms cancels, so a
//   flick past the end does nothing at all.
// - Swipe LEFT to reply (anyone the room lets type): the row follows the finger,
//   an arrow comes in at its right and, past the arm point, an accent disc fills
//   behind it with one light tick; letting go there starts the reply
//   (replySwipe.ts holds the numbers, shared with the iPhone).
//
// One move, one direction: the press is decided once, at the slop. Every press
// on a row holds the list at once (`holdChatRow`), so in a fast chat the row
// pressed is the row acted on, and a tap's click lands on the row it came down
// on: a plain lift leaves the thaw to the list, which waits for that click. It
// listens on the list element natively and keeps its own state, so a moving
// finger re-renders this overlay and nothing else: the pane's re-render storms
// are what once dropped chat swipes. A non-passive touchmove stops the list
// scrolling under the pull, and stops the list's own touch handler, so a swipe
// never pauses chat.
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { Reply } from 'lucide-react';
import { holdChatRow, moderateWithToast } from '../../services/moderation';
import { Logger } from '../../utils/logger';
import { readInsets } from '../nativeInsets';
import { hapticCommit, hapticDestructive, hapticStep, hapticTick } from '../ui/haptics';
import { isThemeDark } from '../../themes';
import {
  GRAB_RATIO,
  inGrabZone,
  inRightGrabZone,
  slideStart,
  stepAction,
  stepKeeping,
  stepTint,
  type SwipeTarget,
  type TouchSlider,
} from './swipeSteps';
import { REPLY_DRAG_START, REPLY_ENGAGE, replyStep, snapBackMs, type ReplyTarget } from './replySwipe';

export type { SwipeTarget, TouchSlider } from './swipeSteps';

interface View {
  /** The row, relative to the list element. */
  top: number;
  left: number;
  width: number;
  height: number;
  slide: number;
  step: number | null;
  offRow: boolean;
  /** Ban's rest is running: a release now cancels. */
  resting: boolean;
}

/** The row a reply swipe moves, relative to the list element. */
interface ReplyView {
  top: number;
  left: number;
  width: number;
  height: number;
}

interface Props {
  listRef: React.RefObject<HTMLElement | null>;
  /** Swiping right to moderate is on here (moderator tools on, not the read-only overlay). */
  modEnabled: boolean;
  /** Swiping left to reply is on here (the room can take a reply). */
  replyEnabled: boolean;
  broadcasterId: string | null;
  /** The chatter behind a row the viewer may swipe, or null. */
  targetFor: (messageId: string) => SwipeTarget | null;
  /** The reply a row would start, or null (`canReplyTo`). */
  replyFor: (messageId: string) => ReplyTarget | null;
  /** The core's slider for a press `startX` px into a row `width` px wide, on `target`'s message. */
  sliderFor: (width: number, startX: number, target: SwipeTarget) => Promise<TouchSlider | null>;
  /** The long-press fan owns the finger once armed. */
  fanArmed: () => boolean;
  /** Gives up the hold menu's pending press: a swipe took the finger. */
  cancelHold: () => void;
  /** A mod swipe started (its one-time hint goes). */
  onDone: () => void;
  /** A reply swipe was let go armed: starts the reply, inside the touch's own handler. */
  onReply: (target: ReplyTarget) => void;
  /** Filled with this swipe's pause ratio for the list's touch rule (`touchPauseIntent`). */
  pauseRatioRef: React.MutableRefObject<() => number | undefined>;
}

export function SwipeModerator({
  listRef,
  modEnabled,
  replyEnabled,
  broadcasterId,
  targetFor,
  replyFor,
  sliderFor,
  fanArmed,
  cancelHold,
  onDone,
  onReply,
  pauseRatioRef,
}: Props) {
  const [view, setView] = useState<View | null>(null);
  const [shown, setShown] = useState<TouchSlider | null>(null);
  const [replyView, setReplyView] = useState<ReplyView | null>(null);
  // The list the overlays draw into, set once it exists, so the arrow is there for a first swipe.
  const [host, setHost] = useState<HTMLElement | null>(null);
  const replyIcon = useRef<HTMLSpanElement | null>(null);
  // Everything the native listeners read, kept current without re-binding them.
  const live = useRef({ modEnabled, replyEnabled, broadcasterId, targetFor, replyFor, sliderFor, fanArmed, cancelHold, onDone, onReply });
  live.current = { modEnabled, replyEnabled, broadcasterId, targetFor, replyFor, sliderFor, fanArmed, cancelHold, onDone, onReply };

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    setHost(list);
    let press: {
      id: number;
      x: number;
      y: number;
      row: HTMLElement;
      messageId: string;
      /** The chatter a mod swipe acts on, or null where none can start. */
      target: SwipeTarget | null;
      /** The reply a left swipe starts, or null where none can. */
      reply: ReplyTarget | null;
      slider: TouchSlider | null;
      /** The core answered for the slider (or no slider was asked for). */
      sliderSettled: boolean;
      /** The press landed in the left grab zone (mod). */
      grab: boolean;
      /** The press landed in the right grab zone (reply). */
      rightGrab: boolean;
    } | null = null;
    /** What the press became at the slop; null while undecided. */
    let kind: 'mod' | 'reply' | null = null;
    let drag = REPLY_DRAG_START;
    let step: number | null = null;
    /** The step zone under the finger, before Ban's rest is applied. */
    let zone: number | null = null;
    let offRow = false;
    let rest: { timer: ReturnType<typeof setTimeout>; anchor: number; armed: boolean } | null = null;
    let rowRect: DOMRect | null = null;
    let listRect: DOMRect | null = null;
    /** Rows still sliding back, with the timer that clears their transition. */
    const settling = new Map<HTMLElement, ReturnType<typeof setTimeout>>();
    /** The one stray click a swipe's release may make, eaten until the next press. */
    let swallow: (() => void) | null = null;

    const body = (row: HTMLElement) => row.querySelector<HTMLElement>('[data-mod-body]');
    const clearTransition = (row: HTMLElement) => {
      const t = settling.get(row);
      if (t) clearTimeout(t);
      settling.delete(row);
      row.style.transition = '';
    };
    const restore = (row: HTMLElement) => {
      clearTransition(row);
      // No slide back while Interface > Motion drops movement.
      const ms = snapBackMs(document.documentElement.dataset.motion);
      if (ms) row.style.transition = `transform ${ms}ms ease-out`;
      row.style.transform = '';
      row.style.maskImage = '';
      const b = body(row);
      if (b) {
        b.style.opacity = '';
        b.style.textDecoration = '';
      }
      if (ms) {
        settling.set(
          row,
          setTimeout(() => {
            settling.delete(row);
            row.style.transition = '';
          }, ms + 20),
        );
      }
    };
    const resetIcon = () => {
      const icon = replyIcon.current;
      if (!icon) return;
      icon.style.opacity = '0';
      icon.style.transform = 'scale(0.6)';
      icon.classList.remove('bg-accent', 'sn-send-ink');
      icon.classList.add('text-textSecondary');
    };
    const stopSwallow = () => {
      swallow?.();
      swallow = null;
    };
    const armSwallow = () => {
      stopSwallow();
      const eat = (e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        stopSwallow();
      };
      const timer = setTimeout(stopSwallow, 400);
      window.addEventListener('click', eat, true);
      swallow = () => {
        clearTimeout(timer);
        window.removeEventListener('click', eat, true);
      };
    };
    const stopRest = () => {
      if (rest) clearTimeout(rest.timer);
      rest = null;
    };
    const resting = () => rest != null && !rest.armed;
    /** The press ends. With `apply` (the finger came up) an armed step or reply runs. */
    const end = (apply: boolean) => {
      const p = press;
      press = null;
      const was = kind;
      kind = null;
      const s = p?.slider ?? null;
      const armedStep = apply && was === 'mod' && !offRow && !resting() && step != null && s ? s.steps[step] : null;
      const replyArmed = apply && was === 'reply' && drag.armed;
      stopRest();
      if (p) {
        if (was) {
          restore(p.row);
          list.removeAttribute('data-row-swipe');
          resetIcon();
          // A swipe makes no tap: the list is let go now, and a stray click from the lift is eaten.
          holdChatRow(p.row, false);
          if (apply) armSwallow();
        } else if (!apply) {
          // Gave way to a scroll, the hold menu, or the browser: let go now.
          holdChatRow(p.row, false);
        }
        // A plain lift (a tap) leaves the thaw to the list, which waits for the tap's click.
        const bid = live.current.broadcasterId;
        if (armedStep && bid && p.target) {
          if (armedStep.action === 'delete') hapticCommit();
          else hapticDestructive();
          void moderateWithToast({
            provider: 'twitch',
            broadcasterId: bid,
            targetUserId: p.target.userId,
            targetName: p.target.name,
            action: stepAction(armedStep, p.messageId),
            useSavedReason: armedStep.action !== 'delete',
          });
        }
        if (replyArmed && p.reply) live.current.onReply(p.reply);
      }
      drag = REPLY_DRAG_START;
      step = null;
      zone = null;
      offRow = false;
      setView(null);
      setReplyView(null);
    };

    const draw = (slide: number) => {
      const s = press?.slider;
      if (!s || !press || !rowRect || !listRect) return;
      const b = body(press.row);
      const lit = step != null && !offRow && !resting();
      if (s.style === 'capsule') {
        press.row.style.transform = `translateX(${slide}px)`;
        // The part slid past the list's edge fades out rather than being cut mid-word.
        press.row.style.maskImage = `linear-gradient(to left, transparent ${slide}px, #000 ${slide + 24}px)`;
      }
      if (s.style === 'preview' && b) {
        b.style.opacity = lit ? '0.45' : '';
        b.style.textDecoration = lit ? `line-through 1.5px ${stepTint(s.steps[step!])}` : '';
      }
      setShown(s);
      setView({
        top: rowRect.top - listRect.top,
        left: rowRect.left - listRect.left,
        width: rowRect.width,
        height: rowRect.height,
        slide,
        step,
        offRow,
        resting: resting(),
      });
    };

    /** The reply swipe follows the finger: style writes only, no render per move. */
    const trackReply = (dx: number) => {
      const p = press;
      if (!p) return;
      const r = replyStep(drag, Math.max(0, -dx));
      drag = r.drag;
      p.row.style.transform = `translateX(${-r.shown}px)`;
      // The part slid past the list's left edge fades out rather than being cut mid-word.
      p.row.style.maskImage = `linear-gradient(to right, transparent ${r.shown}px, #000 ${r.shown + 24}px)`;
      const icon = replyIcon.current;
      if (icon) {
        icon.style.opacity = String(r.progress);
        icon.style.transform = `scale(${0.6 + 0.4 * r.progress})`;
        if (r.armedChanged) {
          icon.classList.toggle('bg-accent', drag.armed);
          icon.classList.toggle('sn-send-ink', drag.armed);
          icon.classList.toggle('text-textSecondary', !drag.armed);
        }
      }
      if (r.armedChanged && drag.armed) hapticTick();
    };

    // The list's touch rule asks this on every move: a swipe that could still start keeps a
    // sideways drag from pausing chat; once the press is decided or gone, the list's own rule.
    pauseRatioRef.current = () => {
      if (!press || kind) return undefined;
      if (press.target && press.grab) return GRAB_RATIO;
      if (press.target || press.reply) return 1;
      return undefined;
    };

    const onDown = (e: PointerEvent) => {
      if (!e.isPrimary || press) return;
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-message-id]');
      const messageId = row?.getAttribute('data-message-id');
      if (!row || !messageId) return;
      const l = live.current;
      const target = l.modEnabled ? l.targetFor(messageId) : null;
      const reply = l.replyEnabled ? l.replyFor(messageId) : null;
      // Hold the list now, on every surface: the row under the thumb is the row a swipe acts on,
      // and the row a tap's click lands on.
      holdChatRow(row, true);
      const r = row.getBoundingClientRect();
      const lr = list.getBoundingClientRect();
      // The system's edge strips (the back gesture, or the cutout and bars) never reach the page,
      // and a list beside the player starts mid-screen: each grab zone starts where both allow.
      const insets = readInsets();
      const leftEdge = Math.max(insets.gestureLeft, insets.left, lr.left);
      const rightEdge = Math.min(lr.right, window.innerWidth - Math.max(insets.gestureRight, insets.right));
      const current = {
        id: e.pointerId,
        x: e.clientX,
        y: e.clientY,
        row,
        messageId,
        target,
        reply,
        slider: null as TouchSlider | null,
        sliderSettled: !target,
        grab: !!target && inGrabZone(e.clientX, leftEdge),
        rightGrab: !!reply && inRightGrabZone(e.clientX, rightEdge),
      };
      press = current;
      if (!target) return;
      trace(`down on ${messageId} at x ${Math.round(e.clientX - r.left)} of ${Math.round(r.width)}; slider requested`);
      void l.sliderFor(r.width, e.clientX - r.left, target).then((s) => {
        trace(s ? `slider ready: enabled ${s.enabled} steps ${s.steps.length} start ${Math.round(s.start)}` : 'slider missing');
        if (press !== current) return;
        current.slider = s;
        current.sliderSettled = true;
        if (kind !== 'mod') return;
        // A slide that latched before the slider came: the core says no and the press ends, or
        // yes and the one-time hint goes.
        if (!s?.enabled) end(false);
        else live.current.onDone();
      });
    };

    const onMove = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!kind) {
        // The fan owns a hold that armed.
        if (live.current.fanArmed()) {
          if (press.target) trace('gave way to the hold menu');
          end(false);
          return;
        }
        const move = slideStart(dx, dy, press.slider?.engage ?? REPLY_ENGAGE, press.grab, {
          mod: !!press.target,
          reply: !!press.reply,
          rightGrab: press.rightGrab,
        });
        // A vertical move is the list's scroll; a sideways one nothing here takes lets the row go.
        if (move === 'scroll' || move === 'none') {
          if (press.target) trace(move === 'scroll' ? 'gave way to a scroll' : 'nothing to swipe that way');
          end(false);
          return;
        }
        if (move === 'wait') return;
        // The core already said this press never slides.
        if (move === 'slide' && press.sliderSettled && !press.slider?.enabled) {
          end(false);
          return;
        }
        kind = move === 'slide' ? 'mod' : 'reply';
        live.current.cancelHold();
        // The pull may leave the list; its release still comes here.
        try {
          list.setPointerCapture(e.pointerId);
        } catch {
          // The pointer is already gone; the next up or cancel ends the pull.
        }
        list.setAttribute('data-row-swipe', '');
        // A slide back still running on this row would make it lag the finger.
        clearTransition(press.row);
        // The newest rows' entrance (Slide or Rise) holds `transform: none` by its fill, and an
        // animation outranks this inline transform: the row would not move under the finger,
        // then jump once a new message pushed it out of the newest three. Its entrance is over
        // (or ends now), so it stays off for good: cleared, it would play again.
        press.row.style.animation = 'none';
        rowRect = press.row.getBoundingClientRect();
        listRect = list.getBoundingClientRect();
        if (kind === 'mod') {
          trace('slider drawn');
          // The gesture is found: the one-time hint goes now, before the chip could meet it (or,
          // with the slider still on its way, once the core says it is on).
          if (press.slider?.enabled) live.current.onDone();
        } else {
          setReplyView({
            top: rowRect.top - listRect.top,
            left: rowRect.left - listRect.left,
            width: rowRect.width,
            height: rowRect.height,
          });
        }
      }
      if (kind === 'reply') {
        trackReply(dx);
        return;
      }
      const s = press.slider;
      // The slider is on its way; until then the finger just pulls.
      if (!s) return;
      const slide = Math.min(Math.max(0, dx), s.end - 0.5);
      offRow = rowRect != null && (e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash);
      // While Ban's rest runs, Ban is the step its edge rule keeps, so a finger
      // trembling on the edge does not start the rest over.
      const here = stepKeeping(s, slide, rest ? s.steps.length - 1 : step);
      let next = here;
      if (next != null && s.steps[next].action === 'ban' && offRow) {
        // Off the row Ban never rests or arms.
        stopRest();
        next = next > 0 ? next - 1 : null;
      } else if (next != null && s.steps[next].action === 'ban') {
        if (!rest || Math.abs(slide - rest.anchor) > s.ban_still) {
          stopRest();
          const anchor = slide;
          rest = {
            anchor,
            armed: false,
            timer: setTimeout(() => {
              if (!rest || !press) return;
              rest.armed = true;
              step = s.steps.length - 1;
              if (!offRow) hapticCommit();
              draw(anchor);
            }, s.ban_dwell_ms),
          };
        }
        if (!rest.armed) next = next > 0 ? next - 1 : null;
      } else {
        stopRest();
      }
      // A buzz marks a new zone under the finger, never Ban arming or disarming (that has its own).
      if (here !== zone) {
        if (here != null && !offRow) hapticStep();
        zone = here;
      }
      step = next;
      draw(slide);
    };

    const onUp = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const s = press.slider;
      if (kind === 'mod' && s && rowRect) offRow = e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash;
      if (kind === 'mod') trace(`released: ${!offRow && !resting() && step != null ? 'ran the armed step' : 'cancelled'}`);
      end(true);
    };
    const onCancel = (e: PointerEvent) => {
      if (press && e.pointerId !== press.id) return;
      // The browser took the touch for a pan of its own: the rows' pan-y should have stopped that.
      if (press?.target || kind === 'reply') trace('the browser took the touch (pointercancel)');
      end(false);
    };
    const onTouchMove = (e: TouchEvent) => {
      if (!kind) return;
      e.preventDefault();
      // The list's own touch handler would read a swipe's jitter as a read-back and pause chat.
      e.stopPropagation();
    };
    // A new press anywhere is the user's own: nothing of an earlier swipe eats it.
    const onAnyDown = () => stopSwallow();
    const onHidden = () => document.visibilityState === 'hidden' && end(false);
    const onBlur = () => end(false);

    list.addEventListener('pointerdown', onDown);
    list.addEventListener('pointermove', onMove);
    list.addEventListener('pointerup', onUp);
    list.addEventListener('pointercancel', onCancel);
    list.addEventListener('touchmove', onTouchMove, { passive: false });
    window.addEventListener('pointerdown', onAnyDown, true);
    window.addEventListener('blur', onBlur);
    document.addEventListener('visibilitychange', onHidden);
    return () => {
      end(false);
      stopSwallow();
      for (const [row, t] of settling) {
        clearTimeout(t);
        row.style.transition = '';
      }
      settling.clear();
      pauseRatioRef.current = () => undefined;
      list.removeEventListener('pointerdown', onDown);
      list.removeEventListener('pointermove', onMove);
      list.removeEventListener('pointerup', onUp);
      list.removeEventListener('pointercancel', onCancel);
      list.removeEventListener('touchmove', onTouchMove);
      window.removeEventListener('pointerdown', onAnyDown, true);
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onHidden);
    };
  }, [listRef, pauseRatioRef]);

  if (!host) return null;
  return (
    <>
      {view && shown && createPortal(<SwipeOverlay view={view} slider={shown} />, host)}
      {createPortal(<ReplyArrow view={replyView} iconRef={replyIcon} />, host)}
    </>
  );
}

/** One line per step of a mod swipe in the app log, so a phone that does nothing can say why. */
function trace(line: string): void {
  Logger.warn('[Swipe]', line);
}

/** The arrow a reply swipe pulls in at the row's right. Always mounted, so a swipe's first move
 *  already has it to write to; the swipe sets its opacity, scale and armed look directly. */
function ReplyArrow({ view, iconRef }: { view: ReplyView | null; iconRef: React.RefObject<HTMLSpanElement | null> }) {
  return (
    <div
      className="pointer-events-none absolute z-30"
      style={view ? { top: view.top, left: view.left, width: view.width, height: view.height } : { display: 'none' }}
      aria-hidden
    >
      <span
        ref={iconRef}
        className="absolute right-3 flex h-8 w-8 items-center justify-center rounded-full text-textSecondary"
        style={{ top: 'calc(50% - 16px)', opacity: 0, transform: 'scale(0.6)' }}
      >
        <Reply size={16} strokeWidth={2.25} />
      </span>
    </div>
  );
}

/** The list fades its top edge; the chip stays below that. */
const LIST_TOP_CLEAR = 30;
/** How far above the row the chip sits: clear of the descenders of the line above, its
 *  words on that line's baseline. */
const CHIP_RISE = 30;
const CHIP_HEIGHT = 28;
/** The chip keeps to the text column's inset, never the screen's edge. */
const CHIP_INSET = 12;
/** The Preview track along the row's bottom, which the chip never covers. */
const TRACK = 3;
/** A fully opaque raised surface for what sits over chat text (the chip and the rail): the
 *  theme's tertiary floor over its background, both over black, so a theme color with any
 *  opacity still paints solid and the message behind never shows through. */
const SOLID_RAISED: CSSProperties = {
  backgroundColor: '#000',
  backgroundImage:
    'linear-gradient(var(--color-background-tertiary), var(--color-background-tertiary)), linear-gradient(var(--color-background), var(--color-background))',
};

/** A step's color as legend text on the solid rail: a light theme darkens it so the light
 *  amber timeouts still read on a light rail. */
function railInk(step: TouchSlider['steps'][number]): string {
  const tint = stepTint(step);
  return isThemeDark() ? tint : `color-mix(in srgb, ${tint} 60%, #000)`;
}

function SwipeOverlay({ view, slider }: { view: View; slider: TouchSlider }) {
  const chipRef = useRef<HTMLDivElement>(null);
  const [chipWidth, setChipWidth] = useState(140);
  const step = view.offRow ? null : view.step;
  const lit = step != null && !view.resting;
  const tint = step != null ? stepTint(slider.steps[step]) : 'var(--color-text-secondary)';
  const words = step == null ? slider.cancel_words : view.resting ? slider.hold_ban_words : slider.steps[step].chip;
  // Measured after each change of words, so the chip centers on the finger.
  useEffect(() => {
    if (chipRef.current) setChipWidth(chipRef.current.offsetWidth);
  }, [words]);
  // The ruler starts where the thumb landed.
  const at = (x: number) => slider.start + x;
  const finger = at(view.slide);
  const chipX = Math.min(Math.max(CHIP_INSET, finger - chipWidth / 2), view.width - chipWidth - CHIP_INSET);
  // Above the row, kept clear of the list's faded top: near the top it comes
  // down over the row rather than under the finger, where the thumb covers
  // it, but never over the track along the row's bottom.
  const chipTop = Math.min(Math.max(-CHIP_RISE, LIST_TOP_CLEAR - view.top), view.height - TRACK - CHIP_HEIGHT - 4);
  return (
    <div
      className="pointer-events-none absolute z-30"
      style={{ top: view.top, left: view.left, width: view.width, height: view.height }}
      aria-live="polite"
    >
      {slider.style === 'rail' ? (
        // Solid: the rail covers the message, and a see-through one let its text clash with the
        // step names.
        // The legend: each name in its own box, the first and last the same distance from their
        // ends and equal gaps between, so "Delete" and "Ban" sit evenly; the lit pill marks the
        // step under the finger.
        <div
          className="absolute inset-x-1 inset-y-1 flex items-center justify-between rounded-[10px] border border-borderSubtle px-1"
          style={SOLID_RAISED}
        >
          {slider.steps.map((s, i) => {
            const on = lit && i === step;
            // glass-exempt: the lit step's tint over the rail, a color cue that must stay translucent.
            const pill = on ? `color-mix(in srgb, ${tint} 22%, transparent)` : undefined;
            return (
              <span
                key={`${s.action}-${s.seconds ?? i}`}
                className="rounded-[9px] px-2 py-0.5 text-[12px] font-semibold"
                style={{ color: railInk(s), background: pill }}
              >
                {s.label}
              </span>
            );
          })}
        </div>
      ) : (
        // glass-exempt: the pull's wash over the message, which must keep the message readable through it.
        // None while a release would cancel (a gray box said nothing); soft at both ends.
        <div
          className="absolute inset-y-0 left-0"
          style={{
            width: finger,
            opacity: step == null ? 0 : 1,
            transition: 'opacity 120ms ease-out',
            background: `linear-gradient(to right, transparent, color-mix(in srgb, ${tint} 18%, transparent) 16px, color-mix(in srgb, ${tint} 18%, transparent) calc(100% - 10px), transparent)`,
          }}
        />
      )}
      {slider.style === 'preview' && (
        <div className="absolute inset-x-0 bottom-0 h-[3px]">
          {slider.steps.map((s, i) => {
            const start = slider.starts[i];
            const zoneEnd = i + 1 < slider.starts.length ? slider.starts[i + 1] : slider.end;
            return (
              <span
                key={`t-${i}`}
                className="absolute top-0 h-[3px] rounded-full"
                style={{ left: at(start) + 1, width: Math.max(2, zoneEnd - start - 2), background: stepTint(s), opacity: start <= view.slide ? 1 : 0.32 }}
              />
            );
          })}
        </div>
      )}
      <div
        ref={chipRef}
        className="absolute flex h-7 items-center justify-center overflow-hidden whitespace-nowrap rounded-full border px-3 text-[13px] font-semibold"
        style={{
          // Solid and lifted off the chat, so the words read over any message on any theme.
          ...SOLID_RAISED,
          boxShadow: '0 1px 2px rgba(0, 0, 0, 0.10), 0 4px 12px rgba(0, 0, 0, 0.14)',
          left: chipX,
          top: chipTop,
          maxWidth: view.width - CHIP_INSET * 2,
          borderColor: view.resting
            ? 'color-mix(in srgb, var(--color-error) 55%, transparent)'
            : `color-mix(in srgb, ${tint} ${step == null ? 35 : 55}%, transparent)`,
          color: lit ? 'var(--color-text-primary)' : 'var(--color-text-secondary)',
        }}
      >
        {view.resting && (
          // glass-exempt: Ban's rest filling the chip behind its words, which must stay readable through it.
          <span
            className="absolute inset-0 bg-error/20"
            style={{ transformOrigin: 'left', animation: `sn-swipe-dwell ${slider.ban_dwell_ms}ms linear forwards` }}
          />
        )}
        <span className="relative truncate">{words}</span>
      </div>
    </div>
  );
}
