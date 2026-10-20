// Swipe right on a message to moderate it: the desktop mod slider with a
// finger, the same on Android as on the iPhone. Rust lays the slider out for
// each press, from where the thumb landed, and says every word with the
// chatter's name in it (`get_touch_mod_slider`, services/touch_mod_slider.rs);
// this follows the finger, draws the chosen style over the row, buzzes on each
// step and on release sends the armed step through the one mod door.
//
// One move, one direction: the pull only goes right. Short of the cancel zone,
// or with the finger strayed `leash` px above or below the message, the chip
// says "Let go to cancel" and a release does nothing. Ban sits at the end of
// the pull and arms only after the finger rests there; a release before it
// arms cancels, so a flick past the end does nothing at all.
//
// The thumb landing on a row the viewer moderates holds the list at once
// (`holdChatRow`) and captures the chatter, so in a fast chat the row pressed
// is the row acted on. It listens on the list element natively and keeps its
// own state, so a moving finger re-renders this overlay and nothing else: the
// pane's re-render storms are what once dropped chat swipes. A non-passive
// touchmove stops the list scrolling under the pull.
import { useEffect, useRef, useState, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { holdChatRow, moderateWithToast } from '../../services/moderation';
import { Logger } from '../../utils/logger';
import { readInsets } from '../nativeInsets';
import { hapticCommit, hapticDestructive, hapticStep } from '../ui/haptics';
import { isThemeDark } from '../../themes';
import { inGrabZone, slideStart, stepAction, stepKeeping, stepTint, type SwipeTarget, type TouchSlider } from './swipeSteps';

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

interface Props {
  listRef: React.RefObject<HTMLElement | null>;
  /** Swiping is on here (moderator tools on, not the read-only overlay). */
  enabled: boolean;
  broadcasterId: string | null;
  /** The chatter behind a row the viewer may swipe, or null. */
  targetFor: (messageId: string) => SwipeTarget | null;
  /** The core's slider for a press `startX` px into a row `width` px wide, on `target`'s message. */
  sliderFor: (width: number, startX: number, target: SwipeTarget) => Promise<TouchSlider | null>;
  /** The long-press fan owns the finger once armed. */
  fanArmed: () => boolean;
  /** A swipe started (the one-time hint goes). */
  onDone: () => void;
}

export function SwipeModerator({ listRef, enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone }: Props) {
  const [view, setView] = useState<View | null>(null);
  const [shown, setShown] = useState<TouchSlider | null>(null);
  // Everything the native listeners read, kept current without re-binding them.
  const live = useRef({ enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone });
  live.current = { enabled, broadcasterId, targetFor, sliderFor, fanArmed, onDone };

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let press: {
      id: number;
      x: number;
      y: number;
      row: HTMLElement;
      messageId: string;
      target: SwipeTarget;
      slider: TouchSlider | null;
      /** The press landed in the grab zone. */
      grab: boolean;
    } | null = null;
    let engaged = false;
    let step: number | null = null;
    /** The step zone under the finger, before Ban's rest is applied. */
    let zone: number | null = null;
    let offRow = false;
    let rest: { timer: ReturnType<typeof setTimeout>; anchor: number; armed: boolean } | null = null;
    let rowRect: DOMRect | null = null;
    let listRect: DOMRect | null = null;

    const body = (row: HTMLElement) => row.querySelector<HTMLElement>('[data-mod-body]');
    const restore = (row: HTMLElement) => {
      row.style.transform = '';
      row.style.maskImage = '';
      row.style.transition = 'transform 220ms ease-out';
      const b = body(row);
      if (b) {
        b.style.opacity = '';
        b.style.textDecoration = '';
      }
      setTimeout(() => (row.style.transition = ''), 240);
    };
    const stopRest = () => {
      if (rest) clearTimeout(rest.timer);
      rest = null;
    };
    const resting = () => rest != null && !rest.armed;
    /** The press ends: the list is let go, and with `apply` an armed step runs. */
    const end = (apply: boolean) => {
      const p = press;
      press = null;
      const s = p?.slider ?? null;
      const armedStep = apply && engaged && !offRow && !resting() && step != null && s ? s.steps[step] : null;
      stopRest();
      if (p) {
        if (engaged) restore(p.row);
        holdChatRow(p.row, false);
        const bid = live.current.broadcasterId;
        if (armedStep && bid) {
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
      }
      engaged = false;
      step = null;
      zone = null;
      offRow = false;
      setView(null);
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

    const onDown = (e: PointerEvent) => {
      if (!e.isPrimary || !live.current.enabled || press) return;
      const row = (e.target as HTMLElement).closest<HTMLElement>('[data-message-id]');
      const messageId = row?.getAttribute('data-message-id');
      const target = messageId ? live.current.targetFor(messageId) : null;
      if (!row || !messageId || !target) return;
      // Hold the list now: the row under the thumb is the row a swipe acts on.
      holdChatRow(row, true);
      const r = row.getBoundingClientRect();
      // The system's left strip (the back gesture, or the cutout and bars) never reaches the page,
      // so the grab zone starts where it ends.
      const insets = readInsets();
      const grab = inGrabZone(e.clientX, Math.max(insets.gestureLeft, insets.left));
      const current = { id: e.pointerId, x: e.clientX, y: e.clientY, row, messageId, target, slider: null as TouchSlider | null, grab };
      press = current;
      trace(`down on ${messageId} at x ${Math.round(e.clientX - r.left)} of ${Math.round(r.width)}; slider requested`);
      void live.current.sliderFor(r.width, e.clientX - r.left, target).then((s) => {
        trace(s ? `slider ready: enabled ${s.enabled} steps ${s.steps.length} start ${Math.round(s.start)}` : 'slider missing');
        if (press === current) current.slider = s;
      });
    };

    const onMove = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const s = press.slider;
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      if (!engaged) {
        // In the grab zone a thumb reaching in from the edge starts a slide sooner and on a
        // steeper angle; everywhere else a slide needs a mostly sideways move.
        const move = slideStart(dx, dy, s?.engage ?? 10, press.grab);
        // A vertical move is the list's scroll, and the fan owns a hold that armed.
        if (live.current.fanArmed() || move === 'scroll') {
          trace(live.current.fanArmed() ? 'gave way to the hold menu' : 'gave way to a scroll');
          end(false);
          return;
        }
        // Sideways and rightward claims the finger, once the core has laid the slider out.
        if (!s?.enabled || move !== 'slide') return;
        engaged = true;
        trace('slider drawn');
        // The pull may leave the list; its release still comes here.
        try {
          list.setPointerCapture(e.pointerId);
        } catch {
          // The pointer is already gone; the next up or cancel ends the pull.
        }
        rowRect = press.row.getBoundingClientRect();
        listRect = list.getBoundingClientRect();
        // The gesture is found: the one-time hint goes now, before the chip could meet it.
        live.current.onDone();
      }
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
      if (s && rowRect) offRow = e.clientY < rowRect.top - s.leash || e.clientY > rowRect.bottom + s.leash;
      if (engaged) trace(`released: ${!offRow && !resting() && step != null ? 'ran the armed step' : 'cancelled'}`);
      end(true);
    };
    const onCancel = (e: PointerEvent) => {
      if (press && e.pointerId !== press.id) return;
      // The browser took the touch for a pan of its own: the rows' pan-y should have stopped that.
      if (press) trace('the browser took the touch (pointercancel)');
      end(false);
    };
    const onTouchMove = (e: TouchEvent) => {
      if (engaged) e.preventDefault();
    };
    const onHidden = () => document.visibilityState === 'hidden' && end(false);
    const onBlur = () => end(false);

    list.addEventListener('pointerdown', onDown);
    list.addEventListener('pointermove', onMove);
    list.addEventListener('pointerup', onUp);
    list.addEventListener('pointercancel', onCancel);
    list.addEventListener('touchmove', onTouchMove, { passive: false });
    window.addEventListener('blur', onBlur);
    document.addEventListener('visibilitychange', onHidden);
    return () => {
      end(false);
      list.removeEventListener('pointerdown', onDown);
      list.removeEventListener('pointermove', onMove);
      list.removeEventListener('pointerup', onUp);
      list.removeEventListener('pointercancel', onCancel);
      list.removeEventListener('touchmove', onTouchMove);
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onHidden);
    };
  }, [listRef]);

  const host = listRef.current;
  if (!view || !shown || !host) return null;
  return createPortal(<SwipeOverlay view={view} slider={shown} />, host);
}

/** One line per step of a swipe in the app log, so a phone that does nothing can say why. */
function trace(line: string): void {
  Logger.warn('[Swipe]', line);
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
